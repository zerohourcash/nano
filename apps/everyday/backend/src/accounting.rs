//! Детерминированная двойная бухгалтерия и локальная единица Bit.

use anyhow::{anyhow, bail};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    h.update(b"everyday/bit-transaction/v1\0");
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    hex::encode(h.finalize())
}

fn entity_guid(conn: &Connection, table: &str, id: i64) -> anyhow::Result<String> {
    let sql = format!("SELECT guid FROM {table} WHERE id=?1");
    if let Some(guid) = conn
        .query_row(&sql, [id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten()
        .filter(|value| !value.is_empty())
    {
        return Ok(guid);
    }
    let guid = uuid::Uuid::new_v4().to_string();
    conn.execute(
        &format!("UPDATE {table} SET guid=?1 WHERE id=?2"),
        params![guid, id],
    )?;
    Ok(guid)
}

fn account_guid(workspace_guid: &str, owner_guid: Option<&str>, code: &str) -> String {
    hash(&[
        "account",
        workspace_guid,
        owner_guid.unwrap_or("system"),
        code,
    ])
}

pub fn wallet(conn: &Connection, workspace: i64, user: i64) -> anyhow::Result<String> {
    if let Some(guid)=conn.query_row("SELECT guid FROM accounting_accounts WHERE workspace_id=?1 AND owner_user_id=?2 AND currency='BIT'",params![workspace,user],|r|r.get(0)).optional()?{return Ok(guid)}
    let member: bool = conn
        .query_row(
            "SELECT 1 FROM user_workspaces WHERE workspace_id=?1 AND user_id=?2",
            params![workspace, user],
            |_| Ok(()),
        )
        .is_ok();
    if !member {
        bail!("пользователь не состоит в организации")
    }
    let workspace_guid = entity_guid(conn, "workspaces", workspace)?;
    let user_guid = entity_guid(conn, "users", user)?;
    let guid = account_guid(&workspace_guid, Some(&user_guid), "BIT");
    conn.execute("INSERT INTO accounting_accounts(guid,workspace_id,owner_user_id,code,name,kind,currency,created_at) VALUES(?1,?2,?3,?4,?5,'wallet','BIT',?6)",params![guid,workspace,user,format!("BIT-{user}"),format!("Bit wallet {user}"),now()])?;
    Ok(guid)
}

fn system_account(conn: &Connection, workspace: i64) -> anyhow::Result<String> {
    if let Some(guid)=conn.query_row("SELECT guid FROM accounting_accounts WHERE workspace_id=?1 AND owner_user_id IS NULL AND code='BIT-ISSUANCE'",[workspace],|r|r.get(0)).optional()?{return Ok(guid)}
    let workspace_guid = entity_guid(conn, "workspaces", workspace)?;
    let guid = account_guid(&workspace_guid, None, "BIT-ISSUANCE");
    conn.execute("INSERT INTO accounting_accounts(guid,workspace_id,owner_user_id,code,name,kind,currency,created_at) VALUES(?1,?2,NULL,'BIT-ISSUANCE','Эмиссия Bit','equity','BIT',?3)",params![guid,workspace,now()])?;
    Ok(guid)
}

#[allow(clippy::too_many_arguments)]
pub fn post(
    conn: &Connection,
    workspace: i64,
    actor: i64,
    kind: &str,
    sender: Option<i64>,
    recipient: i64,
    amount: i64,
    memo: Option<&str>,
    reference: Option<&str>,
) -> anyhow::Result<Value> {
    if amount <= 0 || amount > 9_000_000_000_000_000 {
        bail!("сумма Bit вне допустимого диапазона")
    }
    if !matches!(kind, "transfer" | "sale" | "mint") {
        bail!("неподдерживаемый тип проводки")
    }
    let recipient_account = wallet(conn, workspace, recipient)?;
    let sender_account = match sender {
        Some(user) => wallet(conn, workspace, user)?,
        None => system_account(conn, workspace)?,
    };
    if sender_account == recipient_account {
        bail!("нельзя переводить самому себе")
    }
    let guid = uuid::Uuid::new_v4().to_string();
    let created = now();
    let amount_text = amount.to_string();
    let workspace_guid = entity_guid(conn, "workspaces", workspace)?;
    let actor_guid = entity_guid(conn, "users", actor)?;
    let tx_hash = hash(&[
        &guid,
        &workspace_guid,
        &actor_guid,
        kind,
        &sender_account,
        &recipient_account,
        &amount_text,
        memo.unwrap_or(""),
        reference.unwrap_or(""),
        &created,
    ]);
    conn.execute("INSERT INTO accounting_transactions(guid,workspace_id,actor_user_id,kind,memo,reference,sender_account_guid,amount,tx_hash,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10)",params![guid,workspace,actor,kind,memo,reference,sender_account,amount,tx_hash,created])?;
    conn.execute("INSERT INTO accounting_lines(transaction_guid,account_guid,debit,credit) VALUES(?1,?2,0,?3)",params![guid,sender_account,amount])?;
    conn.execute("INSERT INTO accounting_lines(transaction_guid,account_guid,debit,credit) VALUES(?1,?2,?3,0)",params![guid,recipient_account,amount])?;
    reconcile(conn, workspace)?;
    transaction(conn, &guid).ok_or_else(|| anyhow!("проводка не найдена"))
}

pub fn reconcile(conn: &Connection, workspace: i64) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE accounting_transactions SET status='conflict' WHERE workspace_id=?1",
        [workspace],
    )?;
    conn.execute(
        "UPDATE accounting_transactions SET status='posted' WHERE workspace_id=?1 AND kind='mint'",
        [workspace],
    )?;
    let mut balances = std::collections::HashMap::<String, i64>::new();
    let mut stmt=conn.prepare("SELECT l.account_guid,SUM(l.debit-l.credit) FROM accounting_lines l JOIN accounting_transactions t ON t.guid=l.transaction_guid WHERE t.workspace_id=?1 AND t.kind='mint' GROUP BY l.account_guid")?;
    for row in stmt
        .query_map([workspace], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .flatten()
    {
        balances.insert(row.0, row.1);
    }
    let mut pending = Vec::new();
    let mut stmt=conn.prepare("SELECT t.guid,t.sender_account_guid,t.amount,l.account_guid,t.tx_hash FROM accounting_transactions t JOIN accounting_lines l ON l.transaction_guid=t.guid AND l.debit>0 WHERE t.workspace_id=?1 AND t.kind IN ('transfer','sale') ORDER BY t.tx_hash")?;
    pending.extend(
        stmt.query_map([workspace], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?
        .flatten(),
    );
    loop {
        let mut changed = false;
        for (guid, sender, amount, recipient, _) in &pending {
            let status: String = conn.query_row(
                "SELECT status FROM accounting_transactions WHERE guid=?1",
                [guid],
                |r| r.get(0),
            )?;
            if status == "posted" {
                continue;
            }
            if balances.get(sender).copied().unwrap_or(0) >= *amount {
                *balances.entry(sender.clone()).or_default() -= *amount;
                *balances.entry(recipient.clone()).or_default() += *amount;
                conn.execute(
                    "UPDATE accounting_transactions SET status='posted' WHERE guid=?1",
                    [guid],
                )?;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(())
}

pub fn balance(conn: &Connection, workspace: i64, user: i64) -> anyhow::Result<i64> {
    let Some(account)=conn.query_row("SELECT guid FROM accounting_accounts WHERE workspace_id=?1 AND owner_user_id=?2 AND currency='BIT'",params![workspace,user],|r|r.get::<_,String>(0)).optional()? else{return Ok(0)};
    Ok(conn.query_row("SELECT COALESCE(SUM(CASE WHEN t.status='posted' THEN l.debit-l.credit ELSE 0 END),0) FROM accounting_lines l JOIN accounting_transactions t ON t.guid=l.transaction_guid WHERE l.account_guid=?1",[account],|r|r.get(0))?)
}

pub fn transaction(conn: &Connection, guid: &str) -> Option<Value> {
    conn.query_row("SELECT guid,workspace_id,actor_user_id,kind,memo,reference,sender_account_guid,amount,tx_hash,status,created_at FROM accounting_transactions WHERE guid=?1",[guid],|r|Ok(json!({"guid":r.get::<_,String>(0)?,"workspaceId":r.get::<_,i64>(1)?,"actorId":r.get::<_,i64>(2)?,"kind":r.get::<_,String>(3)?,"memo":r.get::<_,Option<String>>(4)?,"reference":r.get::<_,Option<String>>(5)?,"senderAccountGuid":r.get::<_,Option<String>>(6)?,"amount":r.get::<_,i64>(7)?,"txHash":r.get::<_,String>(8)?,"status":r.get::<_,String>(9)?,"createdAt":r.get::<_,String>(10)?}))).ok()
}

pub fn list(conn: &Connection, workspace: i64) -> Value {
    let mut out = Vec::new();
    if let Ok(mut s)=conn.prepare("SELECT guid FROM accounting_transactions WHERE workspace_id=?1 ORDER BY created_at DESC,guid DESC") {if let Ok(rows)=s.query_map([workspace],|r|r.get::<_,String>(0)){for guid in rows.flatten(){if let Some(v)=transaction(conn,&guid){out.push(v)}}}}
    Value::Array(out)
}

pub fn export(conn: &Connection) -> Value {
    let mut accounts = Vec::new();
    if let Ok(mut s)=conn.prepare("SELECT a.guid,w.guid,u.guid,a.code,a.name,a.kind,a.currency,a.created_at FROM accounting_accounts a JOIN workspaces w ON w.id=a.workspace_id LEFT JOIN users u ON u.id=a.owner_user_id ORDER BY a.guid") {
        if let Ok(rows)=s.query_map([],|r|Ok(json!({"guid":r.get::<_,String>(0)?,"workspaceGuid":r.get::<_,String>(1)?,"ownerGuid":r.get::<_,Option<String>>(2)?,"code":r.get::<_,String>(3)?,"name":r.get::<_,String>(4)?,"kind":r.get::<_,String>(5)?,"currency":r.get::<_,String>(6)?,"createdAt":r.get::<_,String>(7)?}))){accounts.extend(rows.flatten())}
    }
    let mut transactions = Vec::new();
    if let Ok(mut s)=conn.prepare("SELECT t.guid,w.guid,u.guid,t.kind,t.memo,t.reference,t.sender_account_guid,t.amount,t.tx_hash,t.created_at FROM accounting_transactions t JOIN workspaces w ON w.id=t.workspace_id JOIN users u ON u.id=t.actor_user_id ORDER BY t.tx_hash") {
        if let Ok(rows)=s.query_map([],|r|Ok(json!({"guid":r.get::<_,String>(0)?,"workspaceGuid":r.get::<_,String>(1)?,"actorGuid":r.get::<_,String>(2)?,"kind":r.get::<_,String>(3)?,"memo":r.get::<_,Option<String>>(4)?,"reference":r.get::<_,Option<String>>(5)?,"senderAccountGuid":r.get::<_,String>(6)?,"amount":r.get::<_,i64>(7)?,"txHash":r.get::<_,String>(8)?,"createdAt":r.get::<_,String>(9)?}))){transactions.extend(rows.flatten())}
    }
    let mut lines = Vec::new();
    if let Ok(mut s)=conn.prepare("SELECT transaction_guid,account_guid,debit,credit FROM accounting_lines ORDER BY transaction_guid,account_guid") {
        if let Ok(rows)=s.query_map([],|r|Ok(json!({"transactionGuid":r.get::<_,String>(0)?,"accountGuid":r.get::<_,String>(1)?,"debit":r.get::<_,i64>(2)?,"credit":r.get::<_,i64>(3)?}))){lines.extend(rows.flatten())}
    }
    json!({"accounts":accounts,"transactions":transactions,"lines":lines})
}

pub fn import(conn: &Connection, value: &Value) -> anyhow::Result<()> {
    for a in value
        .get("accounts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ws = a
            .get("workspaceGuid")
            .and_then(Value::as_str)
            .and_then(|g| {
                conn.query_row("SELECT id FROM workspaces WHERE guid=?1", [g], |r| {
                    r.get::<_, i64>(0)
                })
                .ok()
            });
        let owner = a.get("ownerGuid").and_then(Value::as_str).and_then(|g| {
            conn.query_row("SELECT id FROM users WHERE guid=?1", [g], |r| {
                r.get::<_, i64>(0)
            })
            .ok()
        });
        let (Some(ws), Some(guid), Some(code), Some(name), Some(kind), Some(currency)) = (
            ws,
            a.get("guid").and_then(Value::as_str),
            a.get("code").and_then(Value::as_str),
            a.get("name").and_then(Value::as_str),
            a.get("kind").and_then(Value::as_str),
            a.get("currency").and_then(Value::as_str),
        ) else {
            continue;
        };
        conn.execute("INSERT OR IGNORE INTO accounting_accounts(guid,workspace_id,owner_user_id,code,name,kind,currency,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![guid,ws,owner,code,name,kind,currency,a.get("createdAt").and_then(Value::as_str).unwrap_or("")])?;
    }
    for t in value
        .get("transactions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let ws = t
            .get("workspaceGuid")
            .and_then(Value::as_str)
            .and_then(|g| {
                conn.query_row("SELECT id FROM workspaces WHERE guid=?1", [g], |r| {
                    r.get::<_, i64>(0)
                })
                .ok()
            });
        let actor = t.get("actorGuid").and_then(Value::as_str).and_then(|g| {
            conn.query_row("SELECT id FROM users WHERE guid=?1", [g], |r| {
                r.get::<_, i64>(0)
            })
            .ok()
        });
        let (
            Some(ws),
            Some(actor),
            Some(guid),
            Some(kind),
            Some(sender),
            Some(amount),
            Some(tx_hash),
            Some(created),
        ) = (
            ws,
            actor,
            t.get("guid").and_then(Value::as_str),
            t.get("kind").and_then(Value::as_str),
            t.get("senderAccountGuid").and_then(Value::as_str),
            t.get("amount").and_then(Value::as_i64),
            t.get("txHash").and_then(Value::as_str),
            t.get("createdAt").and_then(Value::as_str),
        )
        else {
            continue;
        };
        if amount <= 0 || !matches!(kind, "mint" | "transfer" | "sale") {
            bail!("некорректная Bit-транзакция")
        }
        conn.execute("INSERT OR IGNORE INTO accounting_transactions(guid,workspace_id,actor_user_id,kind,memo,reference,sender_account_guid,amount,tx_hash,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10)",params![guid,ws,actor,kind,t.get("memo").and_then(Value::as_str),t.get("reference").and_then(Value::as_str),sender,amount,tx_hash,created])?;
    }
    for l in value
        .get("lines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(tx), Some(account), Some(debit), Some(credit)) = (
            l.get("transactionGuid").and_then(Value::as_str),
            l.get("accountGuid").and_then(Value::as_str),
            l.get("debit").and_then(Value::as_i64),
            l.get("credit").and_then(Value::as_i64),
        ) else {
            continue;
        };
        if debit < 0 || credit < 0 || (debit > 0 && credit > 0) {
            bail!("некорректная строка проводки")
        }
        conn.execute("INSERT OR IGNORE INTO accounting_lines(transaction_guid,account_guid,debit,credit) VALUES(?1,?2,?3,?4)",params![tx,account,debit,credit])?;
    }
    let workspaces: Vec<i64> = {
        let mut s = conn.prepare("SELECT DISTINCT workspace_id FROM accounting_transactions")?;
        let collected = s.query_map([], |r| r.get(0))?.flatten().collect();
        collected
    };
    for ws in workspaces {
        reconcile(conn, ws)?;
    }
    verify(conn)
}

pub fn verify(conn: &Connection) -> anyhow::Result<()> {
    let bad:i64=conn.query_row("SELECT count(*) FROM (SELECT t.guid,t.amount,SUM(l.debit) d,SUM(l.credit) c,COUNT(*) n FROM accounting_transactions t LEFT JOIN accounting_lines l ON l.transaction_guid=t.guid GROUP BY t.guid HAVING d!=c OR d!=t.amount OR n!=2)",[],|r|r.get(0))?;
    if bad != 0 {
        bail!("обнаружена несбалансированная бухгалтерская проводка")
    }
    let mut stmt=conn.prepare("SELECT t.guid,w.guid,u.guid,t.kind,t.sender_account_guid,t.amount,t.memo,t.reference,t.created_at,t.tx_hash,l.account_guid FROM accounting_transactions t JOIN workspaces w ON w.id=t.workspace_id JOIN users u ON u.id=t.actor_user_id JOIN accounting_lines l ON l.transaction_guid=t.guid AND l.debit>0")?;
    for row in stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
            ))
        })?
        .flatten()
    {
        let amount = row.5.to_string();
        let expected = hash(&[
            &row.0,
            &row.1,
            &row.2,
            &row.3,
            &row.4,
            &row.10,
            &amount,
            row.6.as_deref().unwrap_or(""),
            row.7.as_deref().unwrap_or(""),
            &row.8,
        ]);
        if expected != row.9 {
            bail!("хэш Bit-транзакции не совпал")
        }
    }
    let unlinked:i64=conn.query_row("SELECT count(*) FROM accounting_transactions t WHERE NOT EXISTS(SELECT 1 FROM history_entries h WHERE h.workspace_id=t.workspace_id AND h.to_label=t.tx_hash AND h.type=CASE t.kind WHEN 'mint' THEN 'bit_mint' WHEN 'sale' THEN 'bit_sale' ELSE 'bit_transfer' END)",[],|r|r.get(0))?;
    if unlinked != 0 {
        bail!("Bit-транзакция не связана с подписанной летописью")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn setup() -> (Connection, PathBuf, i64, i64, i64) {
        let path =
            std::env::temp_dir().join(format!("everyday-accounting-{}.db", uuid::Uuid::new_v4()));
        let db = crate::db::open(&path).unwrap();
        db.execute("INSERT INTO workspaces(name,internal_id_prefix,created_at,guid) VALUES('Test','T-',?1,?2)",params![now(),uuid::Uuid::new_v4().to_string()]).unwrap();
        let ws = db.last_insert_rowid();
        let mut users = Vec::new();
        for n in 1..=3 {
            db.execute("INSERT INTO users(full_name,phone,status,role_rights,created_at,guid) VALUES(?1,?2,'active',?3,?4,?5)",params![format!("U{n}"),format!("+7000000000{n}"),crate::db::owner_rights().to_string(),now(),uuid::Uuid::new_v4().to_string()]).unwrap();
            let id = db.last_insert_rowid();
            db.execute(
                "INSERT INTO user_workspaces(user_id,workspace_id,rights_json) VALUES(?1,?2,?3)",
                params![id, ws, crate::db::owner_rights().to_string()],
            )
            .unwrap();
            users.push(id)
        }
        (db, path, ws, users[0], users[1])
    }

    fn bind_ledger(db: &Connection, ws: i64, actor: i64, tx: &Value) {
        let op = if tx["kind"] == "mint" {
            "bit_mint"
        } else {
            "bit_transfer"
        };
        crate::ledger::append(
            db,
            ws,
            actor,
            None,
            op,
            None,
            tx["txHash"].as_str(),
            Some(tx["amount"].as_i64().unwrap() as f64),
            None,
        )
        .unwrap();
    }

    #[test]
    fn bit_is_balanced_deterministic_and_tamper_evident() {
        let (db, path, ws, owner, member) = setup();
        let minted = post(
            &db,
            ws,
            owner,
            "mint",
            None,
            owner,
            100,
            Some("старт"),
            None,
        )
        .unwrap();
        bind_ledger(&db, ws, owner, &minted);
        let first = post(
            &db,
            ws,
            owner,
            "transfer",
            Some(owner),
            member,
            70,
            None,
            None,
        )
        .unwrap();
        bind_ledger(&db, ws, owner, &first);
        let competing = post(
            &db,
            ws,
            owner,
            "transfer",
            Some(owner),
            member,
            60,
            None,
            None,
        )
        .unwrap();
        bind_ledger(&db, ws, owner, &competing);
        reconcile(&db, ws).unwrap();
        assert_eq!(
            balance(&db, ws, owner).unwrap() + balance(&db, ws, member).unwrap(),
            100
        );
        let posted:i64=db.query_row("SELECT count(*) FROM accounting_transactions WHERE kind='transfer' AND status='posted'",[],|r|r.get(0)).unwrap();
        assert_eq!(
            posted, 1,
            "двойная трата должна оставить одну каноническую ветвь"
        );
        verify(&db).unwrap();
        db.execute(
            "UPDATE accounting_transactions SET amount=amount+1 WHERE guid=?1",
            [minted["guid"].as_str().unwrap()],
        )
        .unwrap();
        assert!(verify(&db).is_err());
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
