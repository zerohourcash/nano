"""End-to-end proof for encrypted online backup and verified restore."""

from __future__ import annotations

import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BACKUP = ROOT / "deploy/meshkeeper-backup.sh"
RESTORE = ROOT / "deploy/meshkeeper-restore.sh"
PASSWORD = "backup-test-password-with-32-chars"


def check(label: str, condition: bool) -> None:
    print(f"[{'OK  ' if condition else 'FAIL'}] {label}")
    if not condition:
        raise RuntimeError(label)


def run(script: Path, args: list[str], env: dict[str, str]) -> subprocess.CompletedProcess:
    return subprocess.run([str(script), *args], cwd=ROOT, env=env,
                          capture_output=True, text=True, timeout=30)


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="meshkeeper-backup-test-") as temp:
        root = Path(temp)
        source = root / "live.db"
        backups = root / "backups"
        restored = root / "restored.db"
        with sqlite3.connect(source) as db:
            db.execute("PRAGMA journal_mode=WAL")
            db.execute("CREATE TABLE proof(id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
            db.execute("INSERT INTO proof(value) VALUES(?)", ("signed inventory survives",))
            db.commit()

        base = {**os.environ, "MESHKEEPER_DB": str(source),
                "MESHKEEPER_BACKUP_DIR": str(backups), "MESHKEEPER_BACKUP_KEEP": "2"}
        base.pop("MESHKEEPER_BACKUP_PASS", None)
        denied = run(BACKUP, [], base)
        check("backup without encryption password is rejected",
              denied.returncode != 0 and "обязателен" in denied.stderr and not list(backups.glob("*")))

        protected = {**base, "MESHKEEPER_BACKUP_PASS": PASSWORD}
        created = run(BACKUP, [], protected)
        archives = list(backups.glob("*.db.gz.enc"))
        check("encrypted online backup is created with mode 0600",
              created.returncode == 0 and len(archives) == 1 and (archives[0].stat().st_mode & 0o777) == 0o600)
        check("backup does not expose SQLite plaintext header",
              archives[0].read_bytes()[:16] != b"SQLite format 3\x00")

        wrong = run(RESTORE, [str(archives[0]), str(restored)],
                    {**base, "MESHKEEPER_BACKUP_PASS": "definitely-wrong-password-000000"})
        check("wrong restore password fails without publishing a database",
              wrong.returncode != 0 and not restored.exists())

        recovered = run(RESTORE, [str(archives[0]), str(restored)], protected)
        check("correct password restores and integrity-checks SQLite",
              recovered.returncode == 0 and restored.is_file() and (restored.stat().st_mode & 0o777) == 0o600)
        with sqlite3.connect(restored) as db:
            value = db.execute("SELECT value FROM proof").fetchone()[0]
            integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
        check("restored database contains the committed record", value == "signed inventory survives")
        check("restored database passes independent integrity_check", integrity == "ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
