//! Content-addressed вложения и возобновляемая chunk-доставка.

use anyhow::{anyhow, bail, Context};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const CHUNK_SIZE: usize = 64 * 1024;
const MAX_BLOB_SIZE: usize = 32 * 1024 * 1024;

fn digest(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn ingest_data_url(conn: &Connection, value: &str) -> anyhow::Result<Option<String>> {
    let Some(rest) = value.strip_prefix("data:") else {
        return Ok(None);
    };
    let (header, encoded) = rest
        .split_once(',')
        .ok_or_else(|| anyhow!("некорректный data URL"))?;
    let mime = header
        .strip_suffix(";base64")
        .ok_or_else(|| anyhow!("поддерживаются только base64 data URL"))?;
    if mime.len() > 100 || !mime.contains('/') {
        bail!("некорректный MIME")
    }
    let data = STANDARD.decode(encoded).context("некорректный base64")?;
    if data.is_empty() || data.len() > MAX_BLOB_SIZE {
        bail!("размер вложения должен быть от 1 байта до 32 МБ")
    }
    let hash = digest(&data);
    conn.execute(
        "INSERT OR IGNORE INTO content_blobs(hash,mime,size,data,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![hash,mime,data.len() as i64,data,chrono::Utc::now().to_rfc3339()],
    )?;
    // Созданный на этом устройстве объект нельзя удалить до явного unpin.
    conn.execute(
        "INSERT OR IGNORE INTO content_pins(hash,reason,created_at) VALUES(?1,'local',?2)",
        params![hash, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(Some(format!("cas:{hash}")))
}

pub fn resolve_url(conn: &Connection, value: &str) -> String {
    let Some(hash) = value.strip_prefix("cas:") else {
        return value.to_string();
    };
    conn.query_row(
        "SELECT mime,data FROM content_blobs WHERE hash=?1",
        [hash],
        |row| {
            let mime: String = row.get(0)?;
            let data: Vec<u8> = row.get(1)?;
            Ok(format!("data:{mime};base64,{}", STANDARD.encode(data)))
        },
    )
    .unwrap_or_else(|_| "/empty-catalog.svg".into())
}

pub fn manifests(conn: &Connection) -> Value {
    let mut out = Vec::new();
    if let Ok(mut statement) =
        conn.prepare("SELECT hash,mime,size FROM content_blobs ORDER BY hash")
    {
        if let Ok(rows) = statement.query_map([], |row| Ok(json!({"hash":row.get::<_,String>(0)?,"mime":row.get::<_,String>(1)?,"size":row.get::<_,i64>(2)?}))) {
            out.extend(rows.flatten());
        }
    }
    Value::Array(out)
}

pub fn backup_data(conn: &Connection) -> Value {
    let mut out = Vec::new();
    if let Ok(mut statement) =
        conn.prepare("SELECT hash,mime,size,data FROM content_blobs ORDER BY hash")
    {
        if let Ok(rows) = statement.query_map([], |row| {
            let data: Vec<u8> = row.get(3)?;
            Ok(json!({
                "hash": row.get::<_, String>(0)?,
                "mime": row.get::<_, String>(1)?,
                "size": row.get::<_, i64>(2)?,
                "data": STANDARD.encode(data),
            }))
        }) {
            out.extend(rows.flatten());
        }
    }
    Value::Array(out)
}

pub fn restore_backup_data(conn: &Connection, entries: &Value) -> anyhow::Result<usize> {
    let mut restored = 0;
    for entry in entries.as_array().into_iter().flatten() {
        let hash = entry
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("нет hash backup blob"))?;
        let mime = entry
            .get("mime")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("нет MIME backup blob"))?;
        let declared = entry
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("нет размера backup blob"))? as usize;
        let data = STANDARD.decode(
            entry
                .get("data")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("нет данных backup blob"))?,
        )?;
        if declared == 0
            || declared != data.len()
            || declared > MAX_BLOB_SIZE
            || digest(&data) != hash
        {
            bail!("backup blob не прошёл размер/SHA-256");
        }
        restored += conn.execute(
            "INSERT OR IGNORE INTO content_blobs(hash,mime,size,data,created_at) VALUES(?1,?2,?3,?4,?5)",
            params![hash,mime,declared as i64,data,chrono::Utc::now().to_rfc3339()],
        )?;
    }
    Ok(restored)
}

pub fn chunk(conn: &Connection, hash: &str, offset: usize) -> anyhow::Result<Value> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("некорректный hash")
    }
    let (mime, data): (String, Vec<u8>) = conn
        .query_row(
            "SELECT mime,data FROM content_blobs WHERE hash=?1",
            [hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow!("blob не найден"))?;
    if offset > data.len() {
        bail!("offset за пределами blob")
    }
    let end = offset.saturating_add(CHUNK_SIZE).min(data.len());
    Ok(
        json!({"hash":hash,"mime":mime,"offset":offset,"totalSize":data.len(),"data":STANDARD.encode(&data[offset..end]),"done":end==data.len()}),
    )
}

pub fn missing(conn: &Connection, manifest: &Value) -> Vec<String> {
    manifest
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("hash").and_then(Value::as_str))
        .filter(|h| {
            conn.query_row(
                "SELECT 1 FROM content_blobs WHERE hash=?1",
                [*h],
                |_| Ok(()),
            )
            .is_err()
        })
        .map(str::to_string)
        .collect()
}

/// Какие объекты этому узлу действительно нужны. Летопись и manifests всегда
/// синхронизируются отдельно, поэтому отсутствие blob не делает узел неполным.
pub fn wanted_missing(conn: &Connection, journal: &Value) -> Vec<String> {
    let mode = mode(conn);
    let mut wanted = std::collections::BTreeSet::new();
    if mode == "full" || mode == "all" {
        for hash in journal
            .get("blobs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("hash").and_then(Value::as_str))
        {
            wanted.insert(hash.to_string());
        }
    } else if mode != "metadata" && mode != "none" {
        // Smart-узел держит только маленькие превью. Оригинал остаётся у
        // создателя/full-node и может быть явно закреплён позднее.
        for hash in journal
            .get("photos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|photo| photo.get("thumbUrl").and_then(Value::as_str))
            .filter_map(|url| url.strip_prefix("cas:"))
        {
            wanted.insert(hash.to_string());
        }
    }
    if let Ok(mut statement) = conn.prepare("SELECT hash FROM content_pins") {
        if let Ok(rows) = statement.query_map([], |row| row.get::<_, String>(0)) {
            wanted.extend(rows.flatten());
        }
    }
    let advertised: std::collections::BTreeSet<String> = journal
        .get("blobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            entry
                .get("hash")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    wanted
        .into_iter()
        .filter(|hash| advertised.contains(hash))
        .filter(|hash| {
            conn.query_row("SELECT 1 FROM content_blobs WHERE hash=?1", [hash], |_| {
                Ok(())
            })
            .is_err()
        })
        .collect()
}

pub fn mode(conn: &Connection) -> String {
    conn.query_row(
        "SELECT mode FROM content_node_config WHERE singleton=1",
        [],
        |row| row.get(0),
    )
    .unwrap_or_else(|_| {
        match std::env::var("MESHKEEPER_CONTENT_MODE")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "full" | "all" => "full".into(),
            "metadata" | "none" => "metadata".into(),
            _ => "smart".into(),
        }
    })
}

pub fn set_mode(conn: &Connection, value: &str) -> anyhow::Result<()> {
    if !matches!(value, "smart" | "metadata" | "full") {
        bail!("режим: smart, metadata или full")
    }
    conn.execute(
        "INSERT INTO content_node_config(singleton,mode) VALUES(1,?1)
         ON CONFLICT(singleton) DO UPDATE SET mode=excluded.mode",
        [value],
    )?;
    Ok(())
}

pub fn status(conn: &Connection) -> Value {
    let count = |table: &str| {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0)
    };
    let bytes = conn
        .query_row(
            "SELECT coalesce(sum(size),0) FROM content_blobs",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    json!({"mode":mode(conn),"blobs":count("content_blobs"),"pinned":count("content_pins"),"pending":count("blob_downloads"),"bytes":bytes})
}

pub fn pin(conn: &Connection, hash: &str, reason: &str) -> anyhow::Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("некорректный hash")
    }
    conn.execute(
        "INSERT INTO content_pins(hash,reason,created_at) VALUES(?1,?2,?3)
         ON CONFLICT(hash) DO UPDATE SET reason=excluded.reason",
        params![hash, reason, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub fn unpin(conn: &Connection, hash: &str) -> anyhow::Result<()> {
    conn.execute("DELETE FROM content_pins WHERE hash=?1", [hash])?;
    Ok(())
}

pub fn download_offset(conn: &Connection, hash: &str) -> usize {
    conn.query_row(
        "SELECT length(data) FROM blob_downloads WHERE hash=?1",
        [hash],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
    .max(0) as usize
}

pub fn accept_chunk(conn: &Connection, p: &Value) -> anyhow::Result<bool> {
    let hash = p
        .get("hash")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("нет hash"))?;
    let mime = p
        .get("mime")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("нет MIME"))?;
    let offset = p
        .get("offset")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("нет offset"))? as usize;
    let total = p
        .get("totalSize")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("нет размера"))? as usize;
    let bytes = STANDARD.decode(
        p.get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("нет data"))?,
    )?;
    if total == 0
        || total > MAX_BLOB_SIZE
        || bytes.len() > CHUNK_SIZE
        || offset > total
        || offset + bytes.len() > total
    {
        bail!("некорректный размер chunk")
    }
    let current = download_offset(conn, hash);
    if current != offset {
        bail!("неожиданный offset: ожидался {current}, получен {offset}")
    }
    if offset == 0 {
        conn.execute("INSERT INTO blob_downloads(hash,mime,total_size,data,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(hash) DO UPDATE SET mime=excluded.mime,total_size=excluded.total_size,data=excluded.data,updated_at=excluded.updated_at",params![hash,mime,total as i64,bytes,chrono::Utc::now().to_rfc3339()])?;
    } else {
        let mut accumulated: Vec<u8> = conn.query_row(
            "SELECT data FROM blob_downloads WHERE hash=?1 AND total_size=?2 AND mime=?3",
            params![hash, total as i64, mime],
            |row| row.get(0),
        )?;
        accumulated.extend_from_slice(&bytes);
        let changed=conn.execute("UPDATE blob_downloads SET data=?1,updated_at=?2 WHERE hash=?3 AND total_size=?4 AND mime=?5",params![accumulated,chrono::Utc::now().to_rfc3339(),hash,total as i64,mime])?;
        if changed != 1 {
            bail!("параметры продолжения загрузки изменились")
        }
    }
    let complete = download_offset(conn, hash) == total;
    if complete {
        let data: Vec<u8> = conn.query_row(
            "SELECT data FROM blob_downloads WHERE hash=?1",
            [hash],
            |r| r.get(0),
        )?;
        if digest(&data) != hash {
            conn.execute("DELETE FROM blob_downloads WHERE hash=?1", [hash])?;
            bail!("SHA-256 загруженного blob не совпал")
        }
        conn.execute("INSERT OR IGNORE INTO content_blobs(hash,mime,size,data,created_at) VALUES(?1,?2,?3,?4,?5)",params![hash,mime,total as i64,data,chrono::Utc::now().to_rfc3339()])?;
        conn.execute("DELETE FROM blob_downloads WHERE hash=?1", [hash])?;
    }
    Ok(complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE content_blobs(hash TEXT PRIMARY KEY,mime TEXT NOT NULL,size INTEGER NOT NULL,data BLOB NOT NULL,created_at TEXT NOT NULL);
             CREATE TABLE blob_downloads(hash TEXT PRIMARY KEY,mime TEXT NOT NULL,total_size INTEGER NOT NULL,data BLOB NOT NULL,updated_at TEXT NOT NULL);
             CREATE TABLE content_pins(hash TEXT PRIMARY KEY,reason TEXT NOT NULL,created_at TEXT NOT NULL);
             CREATE TABLE content_node_config(singleton INTEGER PRIMARY KEY,mode TEXT NOT NULL);",
        ).unwrap();
        db
    }

    #[test]
    fn data_urls_are_deduplicated_by_raw_content() {
        let db = database();
        let restored = database();
        let first = ingest_data_url(&db, "data:image/png;base64,QUJD")
            .unwrap()
            .unwrap();
        let second = ingest_data_url(&db, "data:image/png;base64,QUJD")
            .unwrap()
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(resolve_url(&db, &first), "data:image/png;base64,QUJD");
        assert_eq!(manifests(&db).as_array().unwrap().len(), 1);
        let backup = backup_data(&db);
        assert_eq!(restore_backup_data(&restored, &backup).unwrap(), 1);
        assert_eq!(resolve_url(&restored, &first), "data:image/png;base64,QUJD");
        let mut corrupted = backup;
        corrupted[0]["data"] = json!("AAAA");
        assert!(restore_backup_data(&restored, &corrupted).is_err());
    }

    #[test]
    fn chunks_resume_and_publish_only_after_hash_verification() {
        let source = database();
        let target = database();
        let raw = vec![42_u8; CHUNK_SIZE + 7];
        let data_url = format!(
            "data:application/octet-stream;base64,{}",
            STANDARD.encode(&raw)
        );
        let cas = ingest_data_url(&source, &data_url).unwrap().unwrap();
        let hash = cas.strip_prefix("cas:").unwrap();
        let first = chunk(&source, hash, 0).unwrap();
        assert!(!accept_chunk(&target, &first).unwrap());
        assert_eq!(download_offset(&target, hash), CHUNK_SIZE);
        assert!(target
            .query_row("SELECT 1 FROM content_blobs", [], |_| Ok(()))
            .is_err());
        let second = chunk(&source, hash, CHUNK_SIZE).unwrap();
        assert!(accept_chunk(&target, &second).unwrap());
        assert_eq!(resolve_url(&target, &cas), data_url);

        let evil = database();
        let mut tampered = first;
        tampered["hash"] = json!("0".repeat(64));
        assert!(!accept_chunk(&evil, &tampered).unwrap());
        let mut tail = second;
        tail["hash"] = json!("0".repeat(64));
        assert!(accept_chunk(&evil, &tail).is_err());
        assert!(evil
            .query_row("SELECT 1 FROM content_blobs", [], |_| Ok(()))
            .is_err());
    }

    #[test]
    fn node_modes_select_content_without_copying_the_ledger_payload() {
        let db = database();
        let thumb = "1".repeat(64);
        let original = "2".repeat(64);
        let document = "3".repeat(64);
        let journal = json!({
            "photos":[{"thumbUrl":format!("cas:{thumb}"),"url":format!("cas:{original}")}],
            "documents":[{"url":format!("cas:{document}")}],
            "blobs":[
                {"hash":thumb,"mime":"image/webp","size":10},
                {"hash":original,"mime":"image/jpeg","size":1000},
                {"hash":document,"mime":"application/pdf","size":2000}
            ]
        });
        assert_eq!(wanted_missing(&db, &journal), vec!["1".repeat(64)]);
        set_mode(&db, "metadata").unwrap();
        assert!(wanted_missing(&db, &journal).is_empty());
        pin(&db, &document, "explicit").unwrap();
        assert_eq!(wanted_missing(&db, &journal), vec![document.clone()]);
        set_mode(&db, "full").unwrap();
        assert_eq!(
            wanted_missing(&db, &journal),
            vec![thumb, original, document]
        );
    }
}
