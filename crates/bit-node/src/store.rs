use anyhow::{Context, Result};
use bit_core::{Block, Hash, Identity, Ledger};
use rand::{RngCore, rngs::OsRng};
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Store {
    conn: Connection,
    identity_path: PathBuf,
}
impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let conn = Connection::open(dir.join("bit.db"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS blocks (position INTEGER PRIMARY KEY AUTOINCREMENT, hash BLOB NOT NULL UNIQUE, data BLOB NOT NULL); CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value BLOB NOT NULL); CREATE TABLE IF NOT EXISTS device_invites (token_hash BLOB PRIMARY KEY, expires_ms INTEGER NOT NULL, used_ms INTEGER); CREATE TABLE IF NOT EXISTS devices (account BLOB PRIMARY KEY, name TEXT NOT NULL, enrolled_ms INTEGER NOT NULL, revoked_ms INTEGER); CREATE TABLE IF NOT EXISTS device_sessions (token_hash BLOB PRIMARY KEY, account BLOB NOT NULL REFERENCES devices(account), expires_ms INTEGER NOT NULL, revoked_ms INTEGER); CREATE TABLE IF NOT EXISTS auth_audit (position INTEGER PRIMARY KEY AUTOINCREMENT, at_ms INTEGER NOT NULL, action TEXT NOT NULL, account BLOB, detail TEXT NOT NULL);")?;
        Ok(Self {
            conn,
            identity_path: dir.join("identity.key"),
        })
    }
    pub fn identity(&self) -> Result<Identity> {
        if self.identity_path.exists() {
            let b = fs::read(&self.identity_path)?;
            let key: [u8; 32] = b
                .try_into()
                .map_err(|_| anyhow::anyhow!("identity.key must contain 32 bytes"))?;
            return Ok(Identity::from_bytes(&key));
        }
        let id = Identity::generate();
        let tmp = self.identity_path.with_extension("tmp");
        fs::write(&tmp, id.secret_bytes())?;
        set_private(&tmp)?;
        fs::rename(tmp, &self.identity_path)?;
        Ok(id)
    }
    pub fn load(&self) -> Result<Ledger> {
        let mut ledger = Ledger::new();
        let mut q = self
            .conn
            .prepare("SELECT data FROM blocks ORDER BY position")?;
        let rows = q.query_map([], |r| r.get::<_, Vec<u8>>(0))?;
        for row in rows {
            let data = row?;
            let block: Block = postcard::from_bytes(&data).context("decode stored block")?;
            ledger
                .insert(block)
                .context("stored ledger verification failed")?;
        }
        Ok(ledger)
    }
    pub fn save(&mut self, hash: Hash, block: &Block) -> Result<()> {
        let data = postcard::to_allocvec(block)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO blocks(hash,data) VALUES(?1,?2)",
            params![hash.0.as_slice(), data],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn admin_token(&self) -> Result<String> {
        let x = std::env::var("BIT_ADMIN_TOKEN").context("BIT_ADMIN_TOKEN is required")?;
        if x.len() < 24 {
            anyhow::bail!("BIT_ADMIN_TOKEN must be at least 24 characters")
        }
        Ok(x)
    }
    pub fn create_invite(&mut self, now_ms: i64, ttl_ms: i64) -> Result<String> {
        if !(60_000..=7 * 24 * 60 * 60 * 1_000).contains(&ttl_ms) {
            anyhow::bail!("invite TTL is outside allowed range")
        }
        let token = random_token();
        self.conn.execute(
            "INSERT INTO device_invites(token_hash,expires_ms) VALUES(?1,?2)",
            params![token_hash(&token).as_slice(), now_ms.saturating_add(ttl_ms)],
        )?;
        self.audit(now_ms, "invite_created", None, "")?;
        Ok(token)
    }
    pub fn enroll_device(
        &mut self,
        invite: &str,
        account: [u8; 32],
        name: &str,
        now_ms: i64,
        session_ttl_ms: i64,
    ) -> Result<String> {
        if name.trim().is_empty() || name.len() > 200 {
            anyhow::bail!("invalid device name")
        }
        if !(60_000..=30 * 24 * 60 * 60 * 1_000).contains(&session_ttl_ms) {
            anyhow::bail!("session TTL is outside allowed range")
        }
        let invite_hash = token_hash(invite);
        let tx = self.conn.transaction()?;
        let valid: bool = tx
            .query_row(
                "SELECT expires_ms>=?2 AND used_ms IS NULL FROM device_invites WHERE token_hash=?1",
                params![invite_hash.as_slice(), now_ms],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !valid {
            anyhow::bail!("invite is invalid, expired, or already used")
        }
        tx.execute(
            "UPDATE device_invites SET used_ms=?2 WHERE token_hash=?1 AND used_ms IS NULL",
            params![invite_hash.as_slice(), now_ms],
        )?;
        tx.execute(
            "INSERT INTO devices(account,name,enrolled_ms) VALUES(?1,?2,?3)",
            params![account.as_slice(), name.trim(), now_ms],
        )?;
        let session = random_token();
        tx.execute(
            "INSERT INTO device_sessions(token_hash,account,expires_ms) VALUES(?1,?2,?3)",
            params![
                token_hash(&session).as_slice(),
                account.as_slice(),
                now_ms.saturating_add(session_ttl_ms)
            ],
        )?;
        tx.execute("INSERT INTO auth_audit(at_ms,action,account,detail) VALUES(?1,'device_enrolled',?2,?3)",params![now_ms,account.as_slice(),name.trim()])?;
        tx.commit()?;
        Ok(session)
    }
    pub fn authenticate_device(&self, token: &str, now_ms: i64) -> Result<[u8; 32]> {
        let bytes: Vec<u8> = self.conn.query_row("SELECT s.account FROM device_sessions s JOIN devices d ON d.account=s.account WHERE s.token_hash=?1 AND s.expires_ms>=?2 AND s.revoked_ms IS NULL AND d.revoked_ms IS NULL",params![token_hash(token).as_slice(),now_ms],|r|r.get(0)).context("invalid or expired device session")?;
        bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("stored device account is corrupt"))
    }
    pub fn revoke_device(&mut self, account: [u8; 32], now_ms: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE devices SET revoked_ms=?2 WHERE account=?1 AND revoked_ms IS NULL",
            params![account.as_slice(), now_ms],
        )?;
        if changed == 0 {
            anyhow::bail!("active device not found")
        }
        tx.execute(
            "UPDATE device_sessions SET revoked_ms=?2 WHERE account=?1 AND revoked_ms IS NULL",
            params![account.as_slice(), now_ms],
        )?;
        tx.execute(
            "INSERT INTO auth_audit(at_ms,action,account,detail) VALUES(?1,'device_revoked',?2,'')",
            params![now_ms, account.as_slice()],
        )?;
        tx.commit()?;
        Ok(())
    }
    fn audit(&self, at_ms: i64, action: &str, account: Option<&[u8]>, detail: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO auth_audit(at_ms,action,account,detail) VALUES(?1,?2,?3,?4)",
            params![at_ms, action, account, detail],
        )?;
        Ok(())
    }
}
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
fn token_hash(token: &str) -> [u8; 32] {
    *blake3::Hasher::new_derive_key("bit-community/device-session/v1")
        .update(token.as_bytes())
        .finalize()
        .as_bytes()
}
#[cfg(unix)]
fn set_private(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
#[cfg(not(unix))]
fn set_private(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bit_core::{CommunityId, Operation};
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn identity_and_verified_ledger_survive_restart() {
        let dir = tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let identity = store.identity().unwrap();
        let account = identity.account();
        let mut ledger = store.load().unwrap();
        let community = CommunityId(*Uuid::now_v7().as_bytes());
        let block = identity.sign(
            ledger
                .next_unsigned(
                    &identity,
                    Operation::Genesis {
                        community,
                        name: "Test".into(),
                        bit_decimals: 2,
                    },
                    1,
                )
                .unwrap(),
        );
        let hash = ledger.insert(block.clone()).unwrap();
        store.save(hash, &block).unwrap();
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(reopened.identity().unwrap().account(), account);
        assert_eq!(reopened.load().unwrap().community(), Some(community));
    }

    #[test]
    fn corrupted_database_block_fails_closed() {
        let dir = tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO blocks(hash,data) VALUES(?1,?2)",
                params![vec![0u8; 32], vec![1u8, 2, 3]],
            )
            .unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn invite_is_single_use_and_session_expires() {
        let dir = tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let invite = store.create_invite(1_000, 60_000).unwrap();
        let session = store
            .enroll_device(&invite, [7; 32], "Телефон Анны", 2_000, 60_000)
            .unwrap();
        assert_eq!(store.authenticate_device(&session, 2_001).unwrap(), [7; 32]);
        assert!(
            store
                .enroll_device(&invite, [8; 32], "Повтор", 2_002, 60_000)
                .is_err()
        );
        assert!(store.authenticate_device(&session, 62_001).is_err());
    }

    #[test]
    fn expired_invite_and_revoked_device_fail_closed_after_restart() {
        let dir = tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let expired = store.create_invite(1_000, 60_000).unwrap();
        assert!(
            store
                .enroll_device(&expired, [1; 32], "Поздно", 61_001, 60_000)
                .is_err()
        );
        let invite = store.create_invite(2_000, 60_000).unwrap();
        let session = store
            .enroll_device(&invite, [9; 32], "Планшет", 2_001, 60_000)
            .unwrap();
        store.revoke_device([9; 32], 2_002).unwrap();
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        assert!(reopened.authenticate_device(&session, 2_003).is_err());
    }
}
