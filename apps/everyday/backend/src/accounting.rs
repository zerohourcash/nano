//! Детерминированная двойная бухгалтерия и локальная единица Bit.

use anyhow::{anyhow, bail};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

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

/// Проверяет переносимую бухгалтерию до импорта. Подпись mesh-журнала сама по
/// себе удостоверяет только ноду; эта проверка связывает экономический смысл
/// проводки с подписанным пользовательским событием Ledger.
pub fn verify_journal_links(conn: &Connection, journal: &Value) -> anyhow::Result<()> {
    let accounting = journal
        .get("accounting")
        .ok_or_else(|| anyhow!("журнал не содержит бухгалтерскую летопись"))?;
    let transactions = accounting
        .get("transactions")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("нет массива Bit-транзакций"))?;
    let lines = accounting
        .get("lines")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("нет строк Bit-проводок"))?;
    let incoming_history: HashMap<&str, &Value> = journal
        .get("history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| {
            event
                .get("opId")
                .and_then(Value::as_str)
                .map(|hash| (hash, event))
        })
        .collect();
    let transaction_guids: HashSet<&str> = transactions
        .iter()
        .filter_map(|tx| tx.get("guid").and_then(Value::as_str))
        .collect();
    if transaction_guids.len() != transactions.len() {
        bail!("повторяющийся или пустой GUID Bit-транзакции")
    }
    if lines.iter().any(|line| {
        line.get("transactionGuid")
            .and_then(Value::as_str)
            .is_none_or(|guid| !transaction_guids.contains(guid))
    }) {
        bail!("строка Bit-проводки ссылается на неизвестную транзакцию")
    }

    for tx in transactions {
        let required = |name: &str| {
            tx.get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("Bit-транзакция без поля {name}"))
        };
        let guid = required("guid")?;
        let workspace_guid = required("workspaceGuid")?;
        let actor_guid = required("actorGuid")?;
        let kind = required("kind")?;
        let sender = required("senderAccountGuid")?;
        let tx_hash = required("txHash")?;
        let created = required("createdAt")?;
        let amount = tx
            .get("amount")
            .and_then(Value::as_i64)
            .filter(|amount| *amount > 0 && *amount <= 9_000_000_000_000_000)
            .ok_or_else(|| anyhow!("некорректная сумма Bit"))?;
        if !matches!(kind, "mint" | "transfer" | "sale") {
            bail!("неподдерживаемый тип Bit-транзакции")
        }
        let tx_lines: Vec<&Value> = lines
            .iter()
            .filter(|line| line.get("transactionGuid").and_then(Value::as_str) == Some(guid))
            .collect();
        if tx_lines.len() != 2 {
            bail!("Bit-проводка должна содержать ровно две строки")
        }
        let debit = tx_lines
            .iter()
            .find(|line| {
                line.get("debit").and_then(Value::as_i64) == Some(amount)
                    && line.get("credit").and_then(Value::as_i64) == Some(0)
            })
            .and_then(|line| line.get("accountGuid").and_then(Value::as_str))
            .ok_or_else(|| anyhow!("нет корректной дебетовой строки Bit"))?;
        let credit = tx_lines
            .iter()
            .find(|line| {
                line.get("credit").and_then(Value::as_i64) == Some(amount)
                    && line.get("debit").and_then(Value::as_i64) == Some(0)
            })
            .and_then(|line| line.get("accountGuid").and_then(Value::as_str))
            .ok_or_else(|| anyhow!("нет корректной кредитовой строки Bit"))?;
        if sender != credit || sender == debit {
            bail!("счета Bit-проводки не соответствуют отправителю")
        }
        let expected = hash(&[
            guid,
            workspace_guid,
            actor_guid,
            kind,
            sender,
            debit,
            &amount.to_string(),
            tx.get("memo").and_then(Value::as_str).unwrap_or(""),
            tx.get("reference").and_then(Value::as_str).unwrap_or(""),
            created,
        ]);
        if expected != tx_hash {
            bail!("хэш переносимой Bit-транзакции не совпал")
        }
        let expected_type = match kind {
            "mint" => "bit_mint",
            "sale" => "bit_sale",
            _ => "bit_transfer",
        };
        let incoming = incoming_history.values().copied().find(|event| {
            event.get("workspaceGuid").and_then(Value::as_str) == Some(workspace_guid)
                && event.get("actorGuid").and_then(Value::as_str) == Some(actor_guid)
                && event.get("type").and_then(Value::as_str) == Some(expected_type)
                && event.get("toLabel").and_then(Value::as_str) == Some(tx_hash)
        });
        if let Some(event) = incoming {
            let quantity = event.get("quantityDelta").and_then(Value::as_f64);
            if quantity != Some(amount as f64)
                || (kind != "mint"
                    && event.get("fromLabel").and_then(Value::as_str) != Some(sender))
            {
                bail!("сумма или отправитель Bit не совпали с подписанной летописью")
            }
        } else {
            let exists: i64 = conn.query_row(
                "SELECT count(*) FROM history_entries h JOIN workspaces w ON w.id=h.workspace_id JOIN users u ON u.id=h.actor_user_id WHERE w.guid=?1 AND u.guid=?2 AND h.type=?3 AND h.to_label=?4 AND h.quantity_delta=?5",
                params![workspace_guid, actor_guid, expected_type, tx_hash, amount as f64],
                |row| row.get(0),
            )?;
            if exists == 0 {
                bail!("Bit-транзакция не связана с подписанной летописью")
            }
        }
    }
    Ok(())
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
            if tx["kind"] == "mint" {
                Some("BIT-ISSUANCE")
            } else {
                tx["senderAccountGuid"].as_str()
            },
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

    #[test]
    fn portable_bit_meaning_must_match_the_signed_ledger_event() {
        let (db, path, ws, owner, member) = setup();
        let minted = post(&db, ws, owner, "mint", None, owner, 100, None, None).unwrap();
        bind_ledger(&db, ws, owner, &minted);
        let sent = post(
            &db,
            ws,
            owner,
            "transfer",
            Some(owner),
            member,
            25,
            Some("смена"),
            None,
        )
        .unwrap();
        bind_ledger(&db, ws, owner, &sent);
        let journal = crate::sync::export_journal(&db);
        verify_journal_links(&db, &journal).unwrap();

        let mut forged_recipient = journal.clone();
        let transfer_guid = sent["guid"].as_str().unwrap();
        let foreign_account = wallet(&db, ws, owner).unwrap();
        let lines = forged_recipient["accounting"]["lines"]
            .as_array_mut()
            .unwrap();
        let recipient_line = lines
            .iter_mut()
            .find(|line| {
                line["transactionGuid"] == transfer_guid
                    && line["debit"].as_i64().unwrap_or_default() > 0
            })
            .unwrap();
        recipient_line["accountGuid"] = json!(foreign_account);
        assert!(verify_journal_links(&db, &forged_recipient).is_err());

        let mut forged_amount = journal;
        let transaction = forged_amount["accounting"]["transactions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|tx| tx["guid"] == transfer_guid)
            .unwrap();
        transaction["amount"] = json!(26);
        assert!(verify_journal_links(&db, &forged_amount).is_err());
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}
