//! Bounded, redacted local operational diagnostics for the owner console.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

const MAX_EVENTS: i64 = 500;

fn clean(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control() || *ch == ' ')
        .take(max)
        .collect::<String>()
}

pub fn record(
    conn: &Connection,
    severity: &str,
    component: &str,
    code: &str,
    message: &str,
    context: Option<&Value>,
) {
    let severity = match severity {
        "info" | "warning" | "error" | "critical" => severity,
        _ => "error",
    };
    let component = clean(component, 64);
    let code = clean(code, 64);
    let message = clean(message, 500);
    let context = context
        .and_then(|value| serde_json::to_string(value).ok())
        .map(|value| clean(&value, 2_000));
    let now = chrono::Utc::now().to_rfc3339();
    let existing = conn
        .query_row(
            "SELECT id FROM diagnostic_events
             WHERE resolved_at IS NULL AND component=?1 AND code=?2
               AND message=?3 AND COALESCE(context_json,'')=COALESCE(?4,'')
             ORDER BY id DESC LIMIT 1",
            params![component, code, message, context],
            |row| row.get::<_, i64>(0),
        )
        .ok();
    if let Some(id) = existing {
        let _ = conn.execute(
            "UPDATE diagnostic_events SET severity=?2,last_at=?3,count=count+1 WHERE id=?1",
            params![id, severity, now],
        );
    } else {
        let _ = conn.execute(
            "INSERT INTO diagnostic_events(severity,component,code,message,context_json,first_at,last_at,count)
             VALUES(?1,?2,?3,?4,?5,?6,?6,1)",
            params![severity, component, code, message, context, now],
        );
    }
    let _ = conn.execute(
        "DELETE FROM diagnostic_events WHERE id IN (
           SELECT id FROM diagnostic_events ORDER BY last_at DESC,id DESC LIMIT -1 OFFSET ?1
         )",
        [MAX_EVENTS],
    );
}

pub fn resolve(conn: &Connection, component: &str, code: &str, context: Option<&Value>) {
    let context = context.and_then(|value| serde_json::to_string(value).ok());
    let _ = conn.execute(
        "UPDATE diagnostic_events SET resolved_at=?1
         WHERE resolved_at IS NULL AND component=?2 AND code=?3
           AND COALESCE(context_json,'')=COALESCE(?4,'')",
        params![chrono::Utc::now().to_rfc3339(), component, code, context],
    );
}

pub fn list(conn: &Connection) -> Value {
    let mut events = Vec::new();
    if let Ok(mut statement) = conn.prepare(
        "SELECT id,severity,component,code,message,context_json,first_at,last_at,count,resolved_at
         FROM diagnostic_events ORDER BY (resolved_at IS NULL) DESC,last_at DESC,id DESC LIMIT 200",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            let context: Option<String> = row.get(5)?;
            Ok(json!({
                "id":row.get::<_,i64>(0)?, "severity":row.get::<_,String>(1)?,
                "component":row.get::<_,String>(2)?, "code":row.get::<_,String>(3)?,
                "message":row.get::<_,String>(4)?,
                "context":context.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()),
                "firstAt":row.get::<_,String>(6)?, "lastAt":row.get::<_,String>(7)?,
                "count":row.get::<_,i64>(8)?, "resolvedAt":row.get::<_,Option<String>>(9)?,
            }))
        }) {
            events.extend(rows.flatten());
        }
    }
    let unresolved = events
        .iter()
        .filter(|event| event.get("resolvedAt").map(Value::is_null).unwrap_or(true))
        .count();
    json!({"unresolved":unresolved,"events":events})
}

pub fn clear_resolved(conn: &Connection) -> Value {
    let removed = conn
        .execute(
            "DELETE FROM diagnostic_events WHERE resolved_at IS NOT NULL",
            [],
        )
        .unwrap_or(0);
    json!({"ok":true,"removed":removed})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE diagnostic_events(id INTEGER PRIMARY KEY AUTOINCREMENT,severity TEXT NOT NULL,component TEXT NOT NULL,code TEXT NOT NULL,message TEXT NOT NULL,context_json TEXT,first_at TEXT NOT NULL,last_at TEXT NOT NULL,count INTEGER NOT NULL DEFAULT 1,resolved_at TEXT)").unwrap();
        db
    }

    #[test]
    fn repeated_errors_are_bounded_deduplicated_and_resolvable() {
        let db = database();
        let context = json!({"peer":"http://10.0.0.2:8766"});
        record(
            &db,
            "warning",
            "sync",
            "peer",
            "нет связи\n",
            Some(&context),
        );
        record(
            &db,
            "warning",
            "sync",
            "peer",
            "нет связи\n",
            Some(&context),
        );
        let result = list(&db);
        assert_eq!(result["unresolved"], 1);
        assert_eq!(result["events"][0]["count"], 2);
        assert_eq!(result["events"][0]["message"], "нет связи");
        resolve(&db, "sync", "peer", Some(&context));
        assert_eq!(list(&db)["unresolved"], 0);
        assert_eq!(clear_resolved(&db)["removed"], 1);
    }
}
