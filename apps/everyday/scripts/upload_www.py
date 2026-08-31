import os
import posixpath
from pathlib import Path

from runtime_config import connect_ssh, required_env

ROOT = Path(__file__).resolve().parents[1]
LOCAL = Path(os.environ.get("MESHKEEPER_WWW_DIR", ROOT / "dist" / "public")).resolve()
REMOTE = "/var/www/meshkeeper"


def upload(sftp, local, remote):
    try:
        sftp.mkdir(remote)
    except OSError:
        pass
    for name in os.listdir(local):
        lp = os.path.join(local, name)
        rp = posixpath.join(remote, name)
        if os.path.isdir(lp):
            upload(sftp, lp, rp)
        else:
            sftp.put(lp, rp)


if required_env("MESHKEEPER_UPLOAD_CONFIRM") != "UPLOAD":
    raise SystemExit("Set MESHKEEPER_UPLOAD_CONFIRM=UPLOAD to confirm upload")
if not (LOCAL / "index.html").is_file():
    raise SystemExit(f"Built web application not found: {LOCAL}")

c = connect_ssh()
sftp = c.open_sftp()
upload(sftp, str(LOCAL), REMOTE)
sftp.close()
c.close()
print("WWW_OK")
