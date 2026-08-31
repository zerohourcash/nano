use crate::ledger;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    apply_encryption_key(&conn)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    init_schema(&conn)?;
    migrate(&conn)?;
    // Request proofs are an in-process hand-off between HTTP verification and
    // ledger append. They must never survive a crash/restart and be reused by
    // an unrelated operation.
    conn.execute("DELETE FROM pending_device_proofs", [])?;
    if std::env::var("MESHKEEPER_DEMO_DATA").as_deref() == Ok("1") {
        seed_if_empty(&conn)?;
    }
    ledger::verify_all(&conn).context("проверка криптографического журнала")?;
    ledger::verify_chat_links(&conn).context("проверка подписей сообщений")?;
    Ok(conn)
}

/// Подключает ключ шифрования базы, если сборка сделана с SQLCipher.
///
/// Ключ берётся из MESHKEEPER_DB_KEY. Оговорка, которую важно понимать:
/// на сервере ключ лежит на том же диске, что и база, поэтому от чтения
/// диска это не спасает — смысл появляется на локальном узле, где базу
/// уносят вместе с ноутбуком. Для такого случая полное шифрование диска
/// (BitLocker, LUKS) защищает лучше: оно закрывает и WAL, и временные файлы.
#[cfg(feature = "encrypted-db")]
fn apply_encryption_key(conn: &Connection) -> Result<()> {
    let key = std::env::var("MESHKEEPER_DB_KEY").unwrap_or_default();
    if key.is_empty() {
        anyhow::bail!("сборка с шифрованием требует MESHKEEPER_DB_KEY (не короче 16 символов)");
    }
    if key.chars().count() < 16 {
        anyhow::bail!("MESHKEEPER_DB_KEY слишком короткий: минимум 16 символов");
    }
    // Экранируем кавычку: ключ приходит из окружения.
    let escaped = key.replace('\'', "''");
    conn.execute_batch(&format!("PRAGMA key = '{escaped}';"))?;
    // Проверка, что ключ подошёл: на чужой базе запрос упадёт.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|_| anyhow::anyhow!("неверный MESHKEEPER_DB_KEY или база повреждена"))?;
    Ok(())
}

#[cfg(not(feature = "encrypted-db"))]
fn apply_encryption_key(_conn: &Connection) -> Result<()> {
    if std::env::var("MESHKEEPER_DB_KEY").is_ok() {
        eprintln!("MESHKEEPER_DB_KEY задан, но сборка без шифрования — база останется открытой");
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let _ = conn.execute("ALTER TABLE items ADD COLUMN due_at TEXT", []);
    let _ = conn.execute("ALTER TABLE items ADD COLUMN guid TEXT", []);
    let _ = conn.execute("ALTER TABLE items ADD COLUMN calibrated_until TEXT", []);
    let _ = conn.execute("ALTER TABLE items ADD COLUMN min_quantity REAL", []);
    let _ = conn.execute("ALTER TABLE items ADD COLUMN source_system TEXT", []);
    let _ = conn.execute("ALTER TABLE items ADD COLUMN external_id TEXT", []);
    let _ = conn.execute("ALTER TABLE items ADD COLUMN metadata_json TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE items ADD COLUMN organization_node_id INTEGER",
        [],
    );
    let _ = conn.execute("ALTER TABLE users ADD COLUMN guid TEXT", []);
    let _ = conn.execute("ALTER TABLE users ADD COLUMN checkout_policy TEXT", []);
    let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN guid TEXT", []);
    let _ = conn.execute("ALTER TABLE workspaces ADD COLUMN sync_url TEXT", []);
    let _ = conn.execute("ALTER TABLE history_entries ADD COLUMN guid TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE history_entries ADD COLUMN event_version INTEGER NOT NULL DEFAULT 1",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_entries ADD COLUMN request_device_id TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_entries ADD COLUMN request_public_key TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_entries ADD COLUMN request_nonce TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_entries ADD COLUMN request_signature TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_entries ADD COLUMN request_hash TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_entries ADD COLUMN request_timestamp TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE history_entries ADD COLUMN request_path TEXT",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE transfers ADD COLUMN needs_admin INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE transfers ADD COLUMN photo_url TEXT", []);
    let _ = conn.execute("ALTER TABLE history_entries ADD COLUMN photo_url TEXT", []);
    // ТЗ §5: у вложения есть уменьшенная копия и контрольная сумма.
    let _ = conn.execute("ALTER TABLE item_photos ADD COLUMN thumb_url TEXT", []);
    let _ = conn.execute("ALTER TABLE item_photos ADD COLUMN sha256 TEXT", []);
    let _ = conn.execute("ALTER TABLE item_photos ADD COLUMN guid TEXT", []);
    let _ = conn.execute("ALTER TABLE item_documents ADD COLUMN guid TEXT", []);
    let _ = conn.execute("ALTER TABLE item_documents ADD COLUMN mime TEXT", []);
    let _ = conn.execute("ALTER TABLE item_documents ADD COLUMN sha256 TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE item_documents ADD COLUMN author_id INTEGER",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE item_documents ADD COLUMN access_level TEXT NOT NULL DEFAULT 'members'",
        [],
    );
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS content_blobs (
           hash TEXT PRIMARY KEY, mime TEXT NOT NULL, size INTEGER NOT NULL,
           data BLOB NOT NULL, created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS blob_downloads (
           hash TEXT PRIMARY KEY, mime TEXT NOT NULL, total_size INTEGER NOT NULL,
           data BLOB NOT NULL, updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS content_pins (
           hash TEXT PRIMARY KEY, reason TEXT NOT NULL,
           created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS content_node_config (
           singleton INTEGER PRIMARY KEY CHECK(singleton=1),
           mode TEXT NOT NULL CHECK(mode IN ('smart','metadata','full'))
         );
         CREATE TABLE IF NOT EXISTS content_catalog (
           hash TEXT PRIMARY KEY, mime TEXT NOT NULL, size INTEGER NOT NULL,
           updated_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS content_providers (
           hash TEXT NOT NULL, url TEXT NOT NULL, last_seen TEXT NOT NULL,
           PRIMARY KEY(hash,url)
         );",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS trusted_node_keys(
           public_key TEXT PRIMARY KEY, label TEXT, approved_by INTEGER,
           source TEXT NOT NULL, created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS pending_node_keys(
           public_key TEXT PRIMARY KEY, peer_url TEXT, node_name TEXT,
           first_seen TEXT NOT NULL, last_seen TEXT NOT NULL
         );",
    )?;
    if let Ok(public_key) = ledger::node_public_key(conn) {
        conn.execute("INSERT OR IGNORE INTO trusted_node_keys(public_key,label,source,created_at) VALUES(?1,'Этот узел','local',?2)",params![public_key,chrono::Utc::now().to_rfc3339()])?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS accounting_accounts(
           guid TEXT PRIMARY KEY, workspace_id INTEGER NOT NULL, owner_user_id INTEGER,
           code TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL, currency TEXT NOT NULL,
           created_at TEXT NOT NULL, UNIQUE(workspace_id,owner_user_id,currency)
         );
         CREATE TABLE IF NOT EXISTS accounting_transactions(
           guid TEXT PRIMARY KEY, workspace_id INTEGER NOT NULL, actor_user_id INTEGER NOT NULL,
           kind TEXT NOT NULL, memo TEXT, reference TEXT, sender_account_guid TEXT,
           amount INTEGER NOT NULL, tx_hash TEXT NOT NULL UNIQUE, status TEXT NOT NULL,
           created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS accounting_lines(
           transaction_guid TEXT NOT NULL, account_guid TEXT NOT NULL,
           debit INTEGER NOT NULL DEFAULT 0, credit INTEGER NOT NULL DEFAULT 0,
           PRIMARY KEY(transaction_guid,account_guid),
           CHECK(debit>=0 AND credit>=0 AND (debit=0 OR credit=0))
         );",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS knowledge_pages(
           guid TEXT PRIMARY KEY,workspace_id INTEGER NOT NULL,slug TEXT NOT NULL,
           title TEXT NOT NULL,visibility TEXT NOT NULL,current_revision_guid TEXT,
           created_at TEXT NOT NULL,UNIQUE(workspace_id,slug)
         );
         CREATE TABLE IF NOT EXISTS knowledge_revisions(
           guid TEXT PRIMARY KEY,page_guid TEXT NOT NULL,parent_guid TEXT,
           author_user_id INTEGER NOT NULL,title TEXT NOT NULL,visibility TEXT NOT NULL,
           content TEXT NOT NULL,attachments_json TEXT NOT NULL,
           revision_hash TEXT NOT NULL UNIQUE,created_at TEXT NOT NULL
         );",
    )?;
    // ТЗ §8: группа может требовать фото-подтверждение при списании.
    let _ = conn.execute(
        "ALTER TABLE workspaces ADD COLUMN require_writeoff_photo INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE user_workspaces ADD COLUMN rights_json TEXT",
        [],
    );
    let _ = conn.execute("ALTER TABLE user_workspaces ADD COLUMN position TEXT", []);
    let _ = conn.execute("ALTER TABLE user_workspaces ADD COLUMN role_name TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE user_workspaces ADD COLUMN personnel_number TEXT",
        [],
    );
    conn.execute(
        "UPDATE user_workspaces SET rights_json=(SELECT role_rights FROM users WHERE users.id=user_workspaces.user_id) WHERE rights_json IS NULL",
        [],
    )?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS faults (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          item_id INTEGER NOT NULL,
          workspace_id INTEGER NOT NULL,
          author_id INTEGER NOT NULL,
          severity TEXT NOT NULL DEFAULT 'medium',
          description TEXT NOT NULL,
          photo_url TEXT,
          status TEXT NOT NULL DEFAULT 'open',
          resolution TEXT,
          resolver_id INTEGER,
          created_at TEXT NOT NULL,
          resolved_at TEXT
        );
        CREATE TABLE IF NOT EXISTS change_requests (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          item_id INTEGER NOT NULL,
          workspace_id INTEGER NOT NULL,
          author_id INTEGER NOT NULL,
          payload TEXT NOT NULL,
          comment TEXT,
          status TEXT NOT NULL DEFAULT 'pending',
          reason TEXT,
          decided_by INTEGER,
          created_at TEXT NOT NULL,
          decided_at TEXT
        );
        CREATE TABLE IF NOT EXISTS chat_messages (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          guid TEXT NOT NULL UNIQUE,
          workspace_id INTEGER NOT NULL,
          user_id INTEGER NOT NULL,
          text TEXT NOT NULL,
          ledger_hash TEXT UNIQUE,
          created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS organization_nodes (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          guid TEXT NOT NULL UNIQUE,
          workspace_id INTEGER NOT NULL,
          parent_id INTEGER,
          kind TEXT NOT NULL,
          name TEXT NOT NULL,
          tab_label TEXT,
          responsible_user_id INTEGER,
          display_order INTEGER NOT NULL DEFAULT 0,
          color TEXT,
          icon TEXT,
          archived INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          FOREIGN KEY(parent_id) REFERENCES organization_nodes(id)
        );
        CREATE INDEX IF NOT EXISTS organization_nodes_tree_idx
          ON organization_nodes(workspace_id,parent_id,display_order,id);
        CREATE TABLE IF NOT EXISTS kv (
          k TEXT PRIMARY KEY,
          v TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS peers (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          node_id TEXT,
          url TEXT NOT NULL UNIQUE,
          name TEXT,
          last_seen TEXT,
          last_sync TEXT,
          last_error TEXT
        );
        CREATE TABLE IF NOT EXISTS item_holdings (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          item_id INTEGER NOT NULL,
          user_id INTEGER NOT NULL,
          quantity REAL NOT NULL DEFAULT 1,
          due_at TEXT,
          comment TEXT,
          photo_url TEXT,
          created_at TEXT NOT NULL,
          returned_at TEXT
        );
        CREATE TABLE IF NOT EXISTS conflicts (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          workspace_id INTEGER,
          item_id INTEGER,
          item_guid TEXT,
          status TEXT NOT NULL DEFAULT 'open',
          description TEXT NOT NULL,
          left_label TEXT,
          right_label TEXT,
          created_at TEXT NOT NULL,
          resolved_at TEXT,
          resolver_id INTEGER
        );
        CREATE TABLE IF NOT EXISTS login_throttle (
          key TEXT PRIMARY KEY,
          failures INTEGER NOT NULL DEFAULT 0,
          last_failure_at TEXT,
          locked_until TEXT
        );
        CREATE TABLE IF NOT EXISTS sessions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          token_hash TEXT NOT NULL UNIQUE,
          user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          created_at TEXT NOT NULL,
          expires_at TEXT NOT NULL,
          revoked_at TEXT
        );
        CREATE INDEX IF NOT EXISTS sessions_user_active_idx
          ON sessions(user_id, expires_at) WHERE revoked_at IS NULL;
        CREATE TABLE IF NOT EXISTS user_devices (
          device_id TEXT PRIMARY KEY,
          user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
          name TEXT NOT NULL,
          public_key TEXT NOT NULL,
          created_at TEXT NOT NULL,
          last_seen_at TEXT,
          revoked_at TEXT
        );
        CREATE INDEX IF NOT EXISTS user_devices_owner_idx ON user_devices(user_id,revoked_at);
        CREATE TABLE IF NOT EXISTS device_nonces (
          device_id TEXT NOT NULL,
          nonce TEXT NOT NULL,
          used_at TEXT NOT NULL,
          PRIMARY KEY(device_id,nonce)
        );
        CREATE TABLE IF NOT EXISTS pending_device_proofs (
          user_id INTEGER PRIMARY KEY,
          device_id TEXT NOT NULL,
          public_key TEXT NOT NULL,
          nonce TEXT NOT NULL,
          signature TEXT NOT NULL,
          request_hash TEXT NOT NULL,
          request_timestamp TEXT NOT NULL,
          request_path TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS user_workspaces_pair_uq
          ON user_workspaces(user_id, workspace_id);
        CREATE UNIQUE INDEX IF NOT EXISTS inventory_result_pair_uq
          ON inventory_results(session_id, item_id);
        CREATE UNIQUE INDEX IF NOT EXISTS hist_hash_uq ON history_entries(hash);
        CREATE UNIQUE INDEX IF NOT EXISTS items_source_external_uq
          ON items(workspace_id, source_system, external_id)
          WHERE source_system IS NOT NULL AND external_id IS NOT NULL;
        "#,
    )?;
    let _ = conn.execute("ALTER TABLE chat_messages ADD COLUMN guid TEXT", []);
    let _ = conn.execute("ALTER TABLE chat_messages ADD COLUMN ledger_hash TEXT", []);
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS chat_messages_guid_idx ON chat_messages(guid) WHERE guid IS NOT NULL;
         CREATE UNIQUE INDEX IF NOT EXISTS chat_messages_ledger_idx ON chat_messages(ledger_hash) WHERE ledger_hash IS NOT NULL;",
    )?;
    let _ = conn.execute(
        "UPDATE item_photos SET guid=lower(hex(randomblob(16))) WHERE guid IS NULL OR guid=''",
        [],
    );
    let _ = conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS item_photos_guid_uq ON item_photos(guid) WHERE guid IS NOT NULL",
        [],
    );
    let _ = conn.execute(
        "UPDATE item_documents SET guid=lower(hex(randomblob(16))) WHERE guid IS NULL OR guid=''",
        [],
    );
    let _ = conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS item_documents_guid_uq ON item_documents(guid) WHERE guid IS NOT NULL",
        [],
    );
    let mut missing_chat_guids = Vec::new();
    {
        let mut stmt =
            conn.prepare("SELECT id FROM chat_messages WHERE guid IS NULL OR guid=''")?;
        missing_chat_guids.extend(stmt.query_map([], |row| row.get::<_, i64>(0))?.flatten());
    }
    for id in missing_chat_guids {
        conn.execute(
            "UPDATE chat_messages SET guid=?1 WHERE id=?2",
            params![uuid::Uuid::new_v4().to_string(), id],
        )?;
    }
    fill_guids(conn)?;
    let _ = conn.execute_batch(
        r#"
        INSERT INTO statuses (name, workspace_id, type, slug, color, bg)
        SELECT 'На проверке', w.id, 'status', 'needs-check', '#A87C0F', '#FBFCC8'
        FROM workspaces w
        WHERE NOT EXISTS (
          SELECT 1 FROM statuses s WHERE s.workspace_id = w.id AND s.slug = 'needs-check'
        );
        "#,
    );
    Ok(())
}

pub fn ensure_workspace_statuses(conn: &Connection, workspace_id: i64) -> Result<()> {
    for (name, slug, color, bg) in [
        ("В работе", "in-work", "#2E9E5B", "#C8FCD2"),
        ("В ремонте", "in-repair", "#A87C0F", "#FBFCC8"),
        ("На складе", "in-stock", "#5E629B", "#EDEDF7"),
        ("На проверке", "needs-check", "#A87C0F", "#FBFCC8"),
        ("Списан", "written-off", "#D64545", "#FAD8D1"),
    ] {
        conn.execute(
            "INSERT INTO statuses (name, workspace_id, type, slug, color, bg)
             SELECT ?2, ?1, 'status', ?3, ?4, ?5
             WHERE NOT EXISTS (SELECT 1 FROM statuses WHERE workspace_id=?1 AND slug=?3)",
            params![workspace_id, name, slug, color, bg],
        )?;
    }
    Ok(())
}

/// Проставляет недостающие GUID. Вызывается при открытии базы и перед каждой
/// выгрузкой: строка, созданная уже после старта, иначе уехала бы с guid=null,
/// и всё, что на неё ссылается, отбрасывалось бы на приёмной стороне.
pub fn fill_guids(conn: &Connection) -> Result<()> {
    for (table, col) in [
        ("workspaces", "guid"),
        ("users", "guid"),
        ("items", "guid"),
        ("history_entries", "guid"),
        ("item_photos", "guid"),
        ("item_documents", "guid"),
    ] {
        let sql = format!(
            "UPDATE {table} SET {col}=lower(hex(randomblob(16))) WHERE {col} IS NULL OR {col}=''"
        );
        let _ = conn.execute(&sql, []);
    }
    Ok(())
}

/// Права наблюдателя: только просмотр каталога и статусов (ТЗ, роль «Наблюдатель»).
pub fn viewer_rights() -> serde_json::Value {
    serde_json::json!({
        "viewItems": true, "createItems": false, "editItems": false, "deleteItems": false,
        "transferItems": false, "acceptTransfers": false, "writeOff": false, "replenish": false,
        "inventory": false, "viewHistory": false, "viewReports": false, "manageUsers": false,
        "manageWorkspaces": false, "manageStorages": false, "manageSites": false, "manageDictionaries": false,
        "reportFaults": false, "requestChanges": false,
        "viewPhotos": true, "viewDocuments": false, "manageDocuments": false,
        "useBit": false, "viewKnowledge": true, "editKnowledge": false, "viewAccounting": false, "manageAccounting": false, "viewLocation": false,
        "checkoutPolicy": default_checkout_policy()
    })
}

/// Права администратора: каталог, заявки, инвентаризация и справочники,
/// но без управления участниками и пространствами (ТЗ, роль «Администратор»).
pub fn admin_rights() -> serde_json::Value {
    serde_json::json!({
        "viewItems": true, "createItems": true, "editItems": true, "deleteItems": true,
        "transferItems": true, "acceptTransfers": true, "writeOff": true, "replenish": true,
        "inventory": true, "viewHistory": true, "viewReports": true, "manageUsers": false,
        "manageWorkspaces": false, "manageStorages": true, "manageSites": true, "manageDictionaries": true,
        "reportFaults": true, "requestChanges": true,
        "viewPhotos": true, "viewDocuments": true, "manageDocuments": true,
        "useBit": true, "viewKnowledge": true, "editKnowledge": true, "viewAccounting": false, "manageAccounting": false, "viewLocation": true,
        "checkoutPolicy": default_checkout_policy()
    })
}

/// Права, которые выдаёт приглашение с указанной ролью.
pub fn rights_for_role(role: &str) -> serde_json::Value {
    match role.trim().to_lowercase().as_str() {
        "owner" | "владелец" => owner_rights(),
        "admin" | "администратор" => admin_rights(),
        "viewer" | "observer" | "наблюдатель" => viewer_rights(),
        _ => default_rights(),
    }
}

pub fn default_checkout_policy() -> serde_json::Value {
    serde_json::json!({
        "allowedCategoryIds": null,
        "maxHours": null,
        "requireApproval": false,
        "allowNoDueDate": true
    })
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS workspaces (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          timezone TEXT NOT NULL DEFAULT 'Europe/Moscow',
          internal_id_prefix TEXT NOT NULL DEFAULT 'ВН-',
          comment TEXT,
          created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS users (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          full_name TEXT NOT NULL,
          position TEXT,
          phone TEXT NOT NULL UNIQUE,
          avatar_url TEXT,
          status TEXT NOT NULL DEFAULT 'active',
          password_hash TEXT,
          role_rights TEXT,
          pubkey TEXT,
          privkey TEXT,
          created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS user_workspaces (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          user_id INTEGER NOT NULL,
          workspace_id INTEGER NOT NULL,
          rights_json TEXT,
          position TEXT,
          role_name TEXT,
          personnel_number TEXT
        );
        CREATE TABLE IF NOT EXISTS storages (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          responsible_user_id INTEGER,
          workspace_id INTEGER NOT NULL,
          address TEXT
        );
        CREATE TABLE IF NOT EXISTS building_sites (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          responsible_user_id INTEGER,
          workspace_id INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS categories (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          description TEXT,
          workspace_id INTEGER NOT NULL,
          type TEXT NOT NULL DEFAULT 'category'
        );
        CREATE TABLE IF NOT EXISTS brands (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          description TEXT,
          workspace_id INTEGER NOT NULL,
          type TEXT NOT NULL DEFAULT 'brand'
        );
        CREATE TABLE IF NOT EXISTS statuses (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          name TEXT NOT NULL,
          description TEXT,
          workspace_id INTEGER NOT NULL,
          type TEXT NOT NULL DEFAULT 'status',
          slug TEXT NOT NULL DEFAULT 'in-stock',
          color TEXT NOT NULL DEFAULT '#5E629B',
          bg TEXT NOT NULL DEFAULT '#EDEDF7'
        );
        CREATE TABLE IF NOT EXISTS items (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          internal_id TEXT NOT NULL,
          title TEXT NOT NULL,
          category_id INTEGER,
          brand_id INTEGER,
          status_id INTEGER,
          responsible_user_id INTEGER,
          building_site_id INTEGER,
          storage_id INTEGER,
          workspace_id INTEGER NOT NULL,
          serial_number TEXT,
          cost REAL,
          quantitative INTEGER NOT NULL DEFAULT 0,
          quantity REAL,
          unit TEXT,
          comment TEXT,
          qr_code TEXT,
          notify_date TEXT,
          due_at TEXT,
          created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS item_photos (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          item_id INTEGER NOT NULL,
          url TEXT NOT NULL,
          is_title INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS item_documents (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          item_id INTEGER NOT NULL,
          name TEXT NOT NULL,
          url TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS item_comments (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          item_id INTEGER NOT NULL,
          user_id INTEGER NOT NULL,
          text TEXT NOT NULL,
          active INTEGER NOT NULL DEFAULT 1,
          created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS transfers (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          code TEXT,
          item_id INTEGER NOT NULL,
          from_user_id INTEGER NOT NULL,
          to_user_id INTEGER NOT NULL,
          to_storage_id INTEGER,
          building_site_id INTEGER,
          workspace_id INTEGER NOT NULL,
          quantity REAL,
          status TEXT NOT NULL DEFAULT 'pending',
          photo_url TEXT,
          comment TEXT,
          no_confirmation INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL,
          completed_at TEXT
        );
        CREATE TABLE IF NOT EXISTS history_entries (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          workspace_id INTEGER NOT NULL,
          item_id INTEGER,
          type TEXT NOT NULL,
          actor_user_id INTEGER NOT NULL,
          from_label TEXT,
          to_label TEXT,
          quantity_delta REAL,
          comment TEXT,
          prev_hash TEXT,
          hash TEXT NOT NULL,
          signature TEXT,
          pubkey TEXT,
          event_version INTEGER NOT NULL DEFAULT 1,
          request_device_id TEXT,
          request_public_key TEXT,
          request_nonce TEXT,
          request_signature TEXT,
          request_hash TEXT,
          request_timestamp TEXT,
          request_path TEXT,
          created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS inventory_sessions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          number TEXT NOT NULL,
          workspace_id INTEGER NOT NULL,
          status TEXT NOT NULL DEFAULT 'in_progress',
          started_by INTEGER NOT NULL,
          created_at TEXT NOT NULL,
          completed_at TEXT
        );
        CREATE TABLE IF NOT EXISTS inventory_results (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id INTEGER NOT NULL,
          item_id INTEGER NOT NULL,
          expected_qty REAL,
          actual_qty REAL,
          checked INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS notifications (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          user_id INTEGER NOT NULL,
          item_id INTEGER,
          type TEXT NOT NULL,
          title TEXT,
          text TEXT NOT NULL,
          read INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS invites (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          workspace_id INTEGER NOT NULL,
          token TEXT NOT NULL UNIQUE,
          role TEXT NOT NULL DEFAULT 'member',
          created_by INTEGER,
          expires_at TEXT,
          max_uses INTEGER NOT NULL DEFAULT 20,
          used_count INTEGER NOT NULL DEFAULT 0,
          revoked INTEGER NOT NULL DEFAULT 0,
          created_at TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

pub fn default_rights() -> serde_json::Value {
    serde_json::json!({
        "viewItems": true, "createItems": true, "editItems": true, "deleteItems": false,
        "transferItems": true, "acceptTransfers": true, "writeOff": false, "replenish": true,
        "inventory": true, "viewHistory": true, "viewReports": true, "manageUsers": false,
        "manageWorkspaces": false, "manageStorages": false, "manageSites": false, "manageDictionaries": false,
        "reportFaults": true, "requestChanges": true,
        "viewPhotos": true, "viewDocuments": true, "manageDocuments": false,
        "useBit": true, "viewKnowledge": true, "editKnowledge": true, "viewAccounting": false, "manageAccounting": false, "viewLocation": true,
        "checkoutPolicy": {
            "allowedCategoryIds": null,
            "maxHours": null,
            "requireApproval": false,
            "allowNoDueDate": true
        }
    })
}

pub fn owner_rights() -> serde_json::Value {
    serde_json::json!({
        "viewItems": true, "createItems": true, "editItems": true, "deleteItems": true,
        "transferItems": true, "acceptTransfers": true, "writeOff": true, "replenish": true,
        "inventory": true, "viewHistory": true, "viewReports": true, "manageUsers": true,
        "manageWorkspaces": true, "manageStorages": true, "manageSites": true, "manageDictionaries": true,
        "reportFaults": true, "requestChanges": true,
        "viewPhotos": true, "viewDocuments": true, "manageDocuments": true,
        "useBit": true, "viewKnowledge": true, "editKnowledge": true, "viewAccounting": true, "manageAccounting": true, "viewLocation": true,
        "checkoutPolicy": {
            "allowedCategoryIds": null,
            "maxHours": null,
            "requireApproval": false,
            "allowNoDueDate": true
        }
    })
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn seed_if_empty(conn: &Connection) -> Result<()> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))?;
    if n > 0 {
        eprintln!("База уже есть, seed пропущен");
        return Ok(());
    }
    if std::env::var("MESHKEEPER_NO_SEED").ok().as_deref() == Some("1") {
        eprintln!("Пустая база, демо-данные отключены (MESHKEEPER_NO_SEED=1)");
        return Ok(());
    }
    eprintln!("Заполнение демо-данных…");
    let ts = now();
    conn.execute(
        "INSERT INTO workspaces (name, timezone, internal_id_prefix, comment, created_at) VALUES (?1,?2,?3,?4,?5)",
        params!["ООО «СтройМонтаж»", "Europe/Moscow", "ВН-", "Основное рабочее пространство", ts],
    )?;
    let ws1 = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO workspaces (name, timezone, internal_id_prefix, comment, created_at) VALUES (?1,?2,?3,?4,?5)",
        params!["ИП «РемСервис»", "Europe/Moscow", "РС-", "Второе пространство", ts],
    )?;

    let people = [
        (
            "Алексей Кузнецов",
            "Кладовщик",
            "+7 921 555-01-42",
            "/avatar-1.png",
            true,
        ),
        (
            "Марина Орлова",
            "Прораб",
            "+7 921 555-02-17",
            "/avatar-2.png",
            false,
        ),
        (
            "Игорь Савельев",
            "Мастер",
            "+7 921 555-03-88",
            "/avatar-3.png",
            false,
        ),
        (
            "Ольга Демидова",
            "Руководитель",
            "+7 921 555-04-29",
            "/avatar-4.png",
            true,
        ),
        (
            "Павел Ким",
            "Монтажник",
            "+7 921 555-05-63",
            "/avatar-5.png",
            false,
        ),
        (
            "Елена Ветрова",
            "Бухгалтер",
            "+7 921 555-06-91",
            "/avatar-6.png",
            false,
        ),
    ];
    let mut uids = Vec::new();
    for (name, pos, phone, av, owner) in people {
        let rights = if owner {
            owner_rights()
        } else {
            default_rights()
        };
        conn.execute(
            "INSERT INTO users (full_name, position, phone, avatar_url, status, role_rights, created_at)
             VALUES (?1,?2,?3,?4,'active',?5,?6)",
            params![name, pos, phone, av, rights.to_string(), ts],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO user_workspaces (user_id, workspace_id) VALUES (?1,?2)",
            params![id, ws1],
        )?;
        uids.push(id);
    }

    conn.execute(
        "INSERT INTO storages (name, responsible_user_id, workspace_id, address) VALUES (?1,?2,?3,?4)",
        params!["Центральный склад", uids[0], ws1, "СПб, Индустриальный пр. 44"],
    )?;
    let wh1 = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO storages (name, responsible_user_id, workspace_id, address) VALUES (?1,?2,?3,?4)",
        params!["Склад №2", uids[0], ws1, "Пушкин"],
    )?;
    let wh2 = conn.last_insert_rowid();

    let sites = [
        ("ЖК «Северная звезда»", uids[1]),
        ("БЦ «Лиговский 87»", uids[2]),
        ("ТРЦ «Галерея»", uids[1]),
    ];
    let mut site_ids = Vec::new();
    for (name, uid) in sites {
        conn.execute(
            "INSERT INTO building_sites (name, responsible_user_id, workspace_id) VALUES (?1,?2,?3)",
            params![name, uid, ws1],
        )?;
        site_ids.push(conn.last_insert_rowid());
    }

    for name in [
        "Электроинструмент",
        "Измерительный и контрольный инструмент",
        "Оргтехника и компьютеры",
        "Ручной инструмент",
        "Расходные материалы",
    ] {
        conn.execute(
            "INSERT INTO categories (name, workspace_id, type) VALUES (?1,?2,'category')",
            params![name, ws1],
        )?;
    }
    for name in ["Bosch", "Makita", "DeWalt", "Karcher", "Metabo", "Зубр"] {
        conn.execute(
            "INSERT INTO brands (name, workspace_id, type) VALUES (?1,?2,'brand')",
            params![name, ws1],
        )?;
    }
    let st = [
        ("in-work", "В работе", "#2E9E5B", "#C8FCD2"),
        ("in-repair", "В ремонте", "#A87C0F", "#FBFCC8"),
        ("in-stock", "На складе", "#5E629B", "#EDEDF7"),
        ("written-off", "Списан", "#D64545", "#FAD8D1"),
    ];
    let mut st_ids = std::collections::HashMap::new();
    for (slug, name, color, bg) in st {
        conn.execute(
            "INSERT INTO statuses (name, workspace_id, type, slug, color, bg) VALUES (?1,?2,'status',?3,?4,?5)",
            params![name, ws1, slug, color, bg],
        )?;
        st_ids.insert(slug, conn.last_insert_rowid());
    }

    struct T {
        vn: &'static str,
        title: &'static str,
        photo: &'static str,
        cat: i64,
        br: i64,
        st: &'static str,
        site: Option<i64>,
        wh: i64,
        user: Option<i64>,
        material: bool,
        qty: Option<f64>,
        unit: Option<&'static str>,
        qr: bool,
        price: f64,
        serial: Option<&'static str>,
        created: &'static str,
    }
    let tools = [
        T {
            vn: "ВН-0142",
            title: "Перфоратор Bosch GBH 8-45 DV",
            photo: "/tool-bosch-gbh.png",
            cat: 1,
            br: 1,
            st: "in-work",
            site: Some(site_ids[0]),
            wh: wh1,
            user: Some(uids[1]),
            material: false,
            qty: None,
            unit: None,
            qr: true,
            price: 68400.0,
            serial: Some("GBH-8845-2210"),
            created: "2024-03-12T00:00:00Z",
        },
        T {
            vn: "ВН-0087",
            title: "Шуруповёрт аккумуляторный Makita DF333",
            photo: "/tool-makita-df.png",
            cat: 1,
            br: 2,
            st: "in-work",
            site: Some(site_ids[0]),
            wh: wh1,
            user: Some(uids[2]),
            material: false,
            qty: None,
            unit: None,
            qr: true,
            price: 18900.0,
            serial: Some("MK-DF333-7741"),
            created: "2023-11-02T00:00:00Z",
        },
        T {
            vn: "ВН-0201",
            title: "Мойка высокого давления Karcher K5",
            photo: "/tool-karcher-k5.png",
            cat: 1,
            br: 4,
            st: "in-stock",
            site: None,
            wh: wh1,
            user: None,
            material: false,
            qty: None,
            unit: None,
            qr: true,
            price: 32500.0,
            serial: Some("KR-K5-0093"),
            created: "2024-06-21T00:00:00Z",
        },
        T {
            vn: "ВН-0115",
            title: "Торцовочная пила DeWalt DWS780",
            photo: "/tool-dewalt-dws.png",
            cat: 1,
            br: 3,
            st: "in-repair",
            site: None,
            wh: wh2,
            user: None,
            material: false,
            qty: None,
            unit: None,
            qr: false,
            price: 89700.0,
            serial: Some("DW-780-5124"),
            created: "2023-08-14T00:00:00Z",
        },
        T {
            vn: "ВН-0156",
            title: "Ноутбук MSI Modern 15",
            photo: "/tool-msi-laptop.png",
            cat: 3,
            br: 1,
            st: "in-work",
            site: Some(site_ids[1]),
            wh: wh1,
            user: Some(uids[3]),
            material: false,
            qty: None,
            unit: None,
            qr: true,
            price: 74900.0,
            serial: Some("MSI-15-9921"),
            created: "2024-01-30T00:00:00Z",
        },
        T {
            vn: "ВН-0063",
            title: "Лазерный уровень Зубр ЛУ-360",
            photo: "/tool-laser-level.png",
            cat: 2,
            br: 6,
            st: "in-stock",
            site: None,
            wh: wh2,
            user: None,
            material: false,
            qty: None,
            unit: None,
            qr: true,
            price: 12700.0,
            serial: Some("ZB-360-3355"),
            created: "2023-05-19T00:00:00Z",
        },
        T {
            vn: "ВН-0178",
            title: "Углошлифмашина Metabo W 650",
            photo: "/tool-metabo-grinder.png",
            cat: 1,
            br: 5,
            st: "in-work",
            site: Some(site_ids[2]),
            wh: wh1,
            user: Some(uids[4]),
            material: false,
            qty: None,
            unit: None,
            qr: false,
            price: 9800.0,
            serial: Some("MT-W650-1187"),
            created: "2024-04-08T00:00:00Z",
        },
        T {
            vn: "ВН-0231",
            title: "Тряпки ветошь 30×30, упаковка 100 шт",
            photo: "/tool-rags.png",
            cat: 5,
            br: 6,
            st: "in-stock",
            site: None,
            wh: wh1,
            user: None,
            material: true,
            qty: Some(96.0),
            unit: Some("шт"),
            qr: false,
            price: 1450.0,
            serial: None,
            created: "2024-09-03T00:00:00Z",
        },
        T {
            vn: "ВН-0088",
            title: "Шуруповёрт аккумуляторный Makita DF333",
            photo: "/tool-makita-df.png",
            cat: 1,
            br: 2,
            st: "in-stock",
            site: None,
            wh: wh1,
            user: None,
            material: false,
            qty: None,
            unit: None,
            qr: true,
            price: 18900.0,
            serial: Some("MK-DF333-7742"),
            created: "2024-02-11T00:00:00Z",
        },
        T {
            vn: "ВН-0203",
            title: "Мойка высокого давления Karcher K5",
            photo: "/tool-karcher-k5.png",
            cat: 1,
            br: 4,
            st: "in-stock",
            site: None,
            wh: wh2,
            user: None,
            material: false,
            qty: None,
            unit: None,
            qr: true,
            price: 32500.0,
            serial: Some("KR-K5-0095"),
            created: "2024-10-01T00:00:00Z",
        },
    ];
    for t in tools {
        conn.execute(
            "INSERT INTO items (internal_id, title, category_id, brand_id, status_id, responsible_user_id, building_site_id, storage_id, workspace_id, serial_number, cost, quantitative, quantity, unit, qr_code, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                t.vn, t.title, t.cat, t.br, st_ids[t.st], t.user, t.site, t.wh, ws1,
                t.serial, t.price, t.material as i64, t.qty, t.unit,
                if t.qr { Some(t.vn) } else { None },
                t.created
            ],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO item_photos (item_id, url, is_title) VALUES (?1,?2,1)",
            params![id, t.photo],
        )?;
        let _ = ledger::append(
            conn,
            ws1,
            uids[0],
            Some(id),
            "create",
            None,
            Some(t.title),
            None,
            Some("Инструмент добавлен в каталог"),
        );
    }
    eprintln!("Готово.");
    Ok(())
}

pub fn digits_only(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}
