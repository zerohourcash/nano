import os
import sqlite3
from pathlib import Path

from runtime_config import connect_ssh, required_env

if required_env("MESHKEEPER_WIPE_CONFIRM") != "WIPE-RELAY-DATA":
    raise SystemExit("Set MESHKEEPER_WIPE_CONFIRM=WIPE-RELAY-DATA to confirm destructive wipe")

local_db = os.environ.get("MESHKEEPER_LOCAL_DB", "").strip()
if local_db:
    local_path = Path(local_db).expanduser().resolve(strict=True)
    local = sqlite3.connect(str(local_path))
    print("local peers before", list(local.execute("SELECT url FROM peers")))
    local.execute("DELETE FROM peers")
    local.commit()
    print("local peers after", list(local.execute("SELECT url FROM peers")))
    local.close()

c = connect_ssh()
cmd = (
    "systemctl stop meshkeeper; "
    "rm -f /opt/meshkeeper/data/meshkeeper-rs.db "
    "/opt/meshkeeper/data/meshkeeper-rs.db-wal "
    "/opt/meshkeeper/data/meshkeeper-rs.db-shm; "
    "systemctl start meshkeeper; sleep 2; "
    "curl -s http://127.0.0.1:8080/health; echo"
)
stdin, stdout, stderr = c.exec_command(cmd, timeout=30)
print(stdout.read().decode())
print(stderr.read().decode()[-400:])
c.close()
print("WIPE_OK")
