//! Offline-mergeable база знаний: страницы и неизменяемый DAG ревизий.

use anyhow::{anyhow, bail};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn digest(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    h.update(b"everyday/knowledge-revision/v1\0");
    for p in parts {
        h.update((p.len() as u64).to_be_bytes());
        h.update(p.as_bytes())
    }
    hex::encode(h.finalize())
}
fn user_guid(conn: &Connection, id: i64) -> anyhow::Result<String> {
    conn.query_row("SELECT guid FROM users WHERE id=?1", [id], |r| {
        r.get::<_, String>(0)
    })
    .map_err(Into::into)
}
fn workspace_guid(conn: &Connection, id: i64) -> anyhow::Result<String> {
    conn.query_row("SELECT guid FROM workspaces WHERE id=?1", [id], |r| {
        r.get::<_, String>(0)
    })
    .map_err(Into::into)
}

fn normalize_attachments(conn: &Connection, value: &Value) -> anyhow::Result<Value> {
    let entries = value.as_array().cloned().unwrap_or_default();
    if entries.len() > 20 {
        bail!("не более 20 вложений на ревизию")
    }
    let mut out = Vec::new();
    for entry in entries {
        let name = entry.get("name").and_then(Value::as_str).unwrap_or("Файл");
        if name.chars().count() > 200 {
            bail!("слишком длинное имя вложения")
        };
        let source = entry
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("нет URL вложения"))?;
        let stored =
            crate::content::ingest_data_url(conn, source)?.unwrap_or_else(|| source.to_string());
        out.push(
            json!({"name":name,"url":stored,"mime":entry.get("mime").and_then(Value::as_str)}),
        );
    }
    Ok(Value::Array(out))
}

#[allow(clippy::too_many_arguments)]
pub fn save(
    conn: &Connection,
    workspace: i64,
    author: i64,
    slug: &str,
    title: &str,
    content: &str,
    visibility: &str,
    parent: Option<&str>,
    attachments: &Value,
) -> anyhow::Result<Value> {
    if slug.is_empty()
        || slug.len() > 120
        || !slug
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/'))
    {
        bail!("slug содержит недопустимые символы")
    }
    if title.trim().is_empty() || title.chars().count() > 200 {
        bail!("некорректный заголовок")
    }
    if content.len() > 2_000_000 {
        bail!("текст страницы превышает 2 МБ")
    }
    if !matches!(visibility, "members" | "accounting" | "managers") {
        bail!("некорректная видимость")
    }
    let page_guid = conn
        .query_row(
            "SELECT guid FROM knowledge_pages WHERE workspace_id=?1 AND slug=?2",
            params![workspace, slug],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
    conn.execute("INSERT INTO knowledge_pages(guid,workspace_id,slug,title,visibility,created_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(workspace_id,slug) DO UPDATE SET title=excluded.title,visibility=excluded.visibility",params![page_guid,workspace,slug,title,visibility,chrono::Utc::now().to_rfc3339()])?;
    let current: Option<String> = conn
        .query_row(
            "SELECT current_revision_guid FROM knowledge_pages WHERE guid=?1",
            [&page_guid],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let parent = parent.map(str::to_string).or(current);
    if let Some(ref p) = parent {
        let valid = conn
            .query_row(
                "SELECT 1 FROM knowledge_revisions WHERE guid=?1 AND page_guid=?2",
                params![p, page_guid],
                |_| Ok(()),
            )
            .is_ok();
        if !valid {
            bail!("родительская ревизия не принадлежит странице")
        }
    }
    let attachments = normalize_attachments(conn, attachments)?;
    let created = chrono::Utc::now().to_rfc3339();
    let guid = uuid::Uuid::new_v4().to_string();
    let author_guid = user_guid(conn, author)?;
    let ws_guid = workspace_guid(conn, workspace)?;
    let revision_hash = digest(&[
        &guid,
        &ws_guid,
        &page_guid,
        parent.as_deref().unwrap_or(""),
        &author_guid,
        title,
        content,
        visibility,
        &attachments.to_string(),
        &created,
    ]);
    conn.execute("INSERT INTO knowledge_revisions(guid,page_guid,parent_guid,author_user_id,title,visibility,content,attachments_json,revision_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![guid,page_guid,parent,author,title,visibility,content,attachments.to_string(),revision_hash,created])?;
    recompute(conn, &page_guid)?;
    let mut result = page(conn, workspace, slug).ok_or_else(|| anyhow!("страница не найдена"))?;
    result["savedRevisionGuid"] = json!(guid);
    result["savedRevisionHash"] = json!(revision_hash);
    Ok(result)
}

pub fn recompute(conn: &Connection, page_guid: &str) -> anyhow::Result<()> {
    let head:Option<String>=conn.query_row("SELECT r.guid FROM knowledge_revisions r WHERE r.page_guid=?1 AND NOT EXISTS(SELECT 1 FROM knowledge_revisions c WHERE c.parent_guid=r.guid) ORDER BY r.revision_hash LIMIT 1",[page_guid],|r|r.get(0)).ok();
    conn.execute(
        "UPDATE knowledge_pages SET current_revision_guid=?1,
         title=COALESCE((SELECT title FROM knowledge_revisions WHERE guid=?1),title),
         visibility=COALESCE((SELECT visibility FROM knowledge_revisions WHERE guid=?1),visibility)
         WHERE guid=?2",
        params![head, page_guid],
    )?;
    Ok(())
}

fn revision_json(conn: &Connection, guid: &str) -> Option<Value> {
    conn.query_row("SELECT r.guid,r.parent_guid,u.guid,u.full_name,r.content,r.attachments_json,r.revision_hash,r.created_at FROM knowledge_revisions r JOIN users u ON u.id=r.author_user_id WHERE r.guid=?1",[guid],|r|{let raw:String=r.get(5)?;let mut attachments:Value=serde_json::from_str(&raw).unwrap_or_else(|_|json!([]));if let Some(items)=attachments.as_array_mut(){for item in items{if let Some(url)=item.get("url").and_then(Value::as_str){item["url"]=json!(crate::content::resolve_url(conn,url));}}}Ok(json!({"guid":r.get::<_,String>(0)?,"parentGuid":r.get::<_,Option<String>>(1)?,"authorGuid":r.get::<_,String>(2)?,"authorName":r.get::<_,String>(3)?,"content":r.get::<_,String>(4)?,"attachments":attachments,"revisionHash":r.get::<_,String>(6)?,"createdAt":r.get::<_,String>(7)?}))}).ok()
}

pub fn page(conn: &Connection, workspace: i64, slug: &str) -> Option<Value> {
    conn.query_row("SELECT guid,slug,title,visibility,current_revision_guid,created_at FROM knowledge_pages WHERE workspace_id=?1 AND slug=?2",params![workspace,slug],|r|{let guid:String=r.get(0)?;let current:Option<String>=r.get(4)?;let mut heads=Vec::new();if let Ok(mut s)=conn.prepare("SELECT r.guid FROM knowledge_revisions r WHERE r.page_guid=?1 AND NOT EXISTS(SELECT 1 FROM knowledge_revisions c WHERE c.parent_guid=r.guid) ORDER BY r.revision_hash"){if let Ok(rows)=s.query_map([&guid],|x|x.get::<_,String>(0)){heads.extend(rows.flatten())}}Ok(json!({"guid":guid,"slug":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"visibility":r.get::<_,String>(3)?,"currentRevisionGuid":current,"current":current.as_deref().and_then(|id|revision_json(conn,id)),"headGuids":heads,"hasConflict":heads.len()>1,"createdAt":r.get::<_,String>(5)?}))}).ok()
}

pub fn list(conn: &Connection, workspace: i64) -> Value {
    let mut out = Vec::new();
    if let Ok(mut s) =
        conn.prepare("SELECT slug FROM knowledge_pages WHERE workspace_id=?1 ORDER BY title")
    {
        if let Ok(rows) = s.query_map([workspace], |r| r.get::<_, String>(0)) {
            for slug in rows.flatten() {
                if let Some(page) = page(conn, workspace, &slug) {
                    out.push(page)
                }
            }
        }
    }
    Value::Array(out)
}

pub fn export(conn: &Connection) -> Value {
    let mut pages = Vec::new();
    if let Ok(mut s)=conn.prepare("SELECT p.guid,w.guid,p.slug,p.title,p.visibility,p.created_at FROM knowledge_pages p JOIN workspaces w ON w.id=p.workspace_id ORDER BY p.guid"){if let Ok(rows)=s.query_map([],|r|Ok(json!({"guid":r.get::<_,String>(0)?,"workspaceGuid":r.get::<_,String>(1)?,"slug":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"visibility":r.get::<_,String>(4)?,"createdAt":r.get::<_,String>(5)?}))){pages.extend(rows.flatten())}}
    let mut revisions = Vec::new();
    if let Ok(mut s)=conn.prepare("SELECT r.guid,r.page_guid,r.parent_guid,u.guid,r.title,r.visibility,r.content,r.attachments_json,r.revision_hash,r.created_at FROM knowledge_revisions r JOIN users u ON u.id=r.author_user_id ORDER BY r.revision_hash"){if let Ok(rows)=s.query_map([],|r|Ok(json!({"guid":r.get::<_,String>(0)?,"pageGuid":r.get::<_,String>(1)?,"parentGuid":r.get::<_,Option<String>>(2)? ,"authorGuid":r.get::<_,String>(3)?,"title":r.get::<_,String>(4)?,"visibility":r.get::<_,String>(5)?,"content":r.get::<_,String>(6)?,"attachments":serde_json::from_str::<Value>(&r.get::<_,String>(7)?).unwrap_or_else(|_|json!([])),"revisionHash":r.get::<_,String>(8)?,"createdAt":r.get::<_,String>(9)?}))){revisions.extend(rows.flatten())}}
    json!({"pages":pages,"revisions":revisions})
}

pub fn import(conn: &Connection, value: &Value) -> anyhow::Result<()> {
    for p in value
        .get("pages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ws = p
            .get("workspaceGuid")
            .and_then(Value::as_str)
            .and_then(|g| {
                conn.query_row("SELECT id FROM workspaces WHERE guid=?1", [g], |r| {
                    r.get::<_, i64>(0)
                })
                .ok()
            });
        let (Some(ws), Some(guid), Some(slug), Some(title), Some(visibility)) = (
            ws,
            p.get("guid").and_then(Value::as_str),
            p.get("slug").and_then(Value::as_str),
            p.get("title").and_then(Value::as_str),
            p.get("visibility").and_then(Value::as_str),
        ) else {
            continue;
        };
        conn.execute("INSERT OR IGNORE INTO knowledge_pages(guid,workspace_id,slug,title,visibility,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![guid,ws,slug,title,visibility,p.get("createdAt").and_then(Value::as_str).unwrap_or("")])?;
    }
    for r in value
        .get("revisions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let author = r.get("authorGuid").and_then(Value::as_str).and_then(|g| {
            conn.query_row("SELECT id FROM users WHERE guid=?1", [g], |x| {
                x.get::<_, i64>(0)
            })
            .ok()
        });
        let (
            Some(author),
            Some(guid),
            Some(page),
            Some(title),
            Some(visibility),
            Some(content),
            Some(revision_hash),
            Some(created),
        ) = (
            author,
            r.get("guid").and_then(Value::as_str),
            r.get("pageGuid").and_then(Value::as_str),
            r.get("title").and_then(Value::as_str),
            r.get("visibility").and_then(Value::as_str),
            r.get("content").and_then(Value::as_str),
            r.get("revisionHash").and_then(Value::as_str),
            r.get("createdAt").and_then(Value::as_str),
        )
        else {
            continue;
        };
        conn.execute("INSERT OR IGNORE INTO knowledge_revisions(guid,page_guid,parent_guid,author_user_id,title,visibility,content,attachments_json,revision_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![guid,page,r.get("parentGuid").and_then(Value::as_str),author,title,visibility,content,r.get("attachments").cloned().unwrap_or_else(||json!([])).to_string(),revision_hash,created])?;
    }
    let pages: Vec<String> = {
        let mut s = conn.prepare("SELECT guid FROM knowledge_pages")?;
        let v = s.query_map([], |r| r.get(0))?.flatten().collect();
        v
    };
    for p in pages {
        recompute(conn, &p)?
    }
    verify(conn)
}

pub fn verify(conn: &Connection) -> anyhow::Result<()> {
    let mut s=conn.prepare("SELECT r.guid,w.guid,r.page_guid,r.parent_guid,u.guid,r.title,r.content,r.visibility,r.attachments_json,r.created_at,r.revision_hash FROM knowledge_revisions r JOIN knowledge_pages p ON p.guid=r.page_guid JOIN workspaces w ON w.id=p.workspace_id JOIN users u ON u.id=r.author_user_id")?;
    for row in s
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
            ))
        })?
        .flatten()
    {
        let expected = digest(&[
            &row.0,
            &row.1,
            &row.2,
            row.3.as_deref().unwrap_or(""),
            &row.4,
            &row.5,
            &row.6,
            &row.7,
            &row.8,
            &row.9,
        ]);
        if expected != row.10 {
            bail!("хэш ревизии базы знаний не совпал")
        }
        let linked=conn.query_row("SELECT 1 FROM history_entries WHERE type='knowledge_revision' AND from_label=?1 AND to_label=?2",params![row.2,row.10],|_|Ok(())).is_ok();
        if !linked {
            bail!("ревизия базы знаний не связана с летописью")
        }
    }
    Ok(())
}
