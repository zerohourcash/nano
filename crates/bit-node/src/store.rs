use anyhow::{Context, Result};
use bit_core::{Block, Hash, Identity, Ledger};
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
        conn.execute_batch("CREATE TABLE IF NOT EXISTS blocks (position INTEGER PRIMARY KEY AUTOINCREMENT, hash BLOB NOT NULL UNIQUE, data BLOB NOT NULL); CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value BLOB NOT NULL);")?;
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
}
