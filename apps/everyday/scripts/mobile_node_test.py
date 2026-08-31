"""Host proof of the Android topology: private UI + sync-only LAN listener."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "backend" / "target" / "release" / "meshkeeper-node"
TOKEN = "android-host-test-token-at-least-32-characters"


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(url: str, token: str | None = None) -> tuple[int, bytes]:
    req = urllib.request.Request(url)
    if token:
        req.add_header("authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=3) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


def main() -> int:
    if not BINARY.is_file():
        print("release binary отсутствует; выполните cargo build --release")
        return 2
    ui_port, sync_port = free_port(), free_port()
    db = Path(tempfile.gettempdir()) / f"meshkeeper-android-host-{uuid.uuid4().hex}.db"
    env = {
        **os.environ,
        "MESHKEEPER_DB": str(db),
        "MESHKEEPER_WEB_ROOT": str(ROOT / "dist" / "public"),
        "MESHKEEPER_BIND": f"127.0.0.1:{ui_port}",
        "MESHKEEPER_SYNC_BIND": f"127.0.0.1:{sync_port}",
        "MESHKEEPER_SYNC_TOKEN": TOKEN,
        "MESHKEEPER_ADVERTISE_URL": f"http://127.0.0.1:{sync_port}",
        "MESHKEEPER_ALLOW_INSECURE_SYNC": "1",
        "MESHKEEPER_DEMO_DATA": "0",
        "MESHKEEPER_DEMO_LOGIN": "0",
    }
    proc = subprocess.Popen([str(BINARY)], cwd=ROOT, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        for _ in range(100):
            try:
                if request(f"http://127.0.0.1:{ui_port}/health")[0] == 200:
                    break
            except OSError:
                pass
            if proc.poll() is not None:
                print(proc.stderr.read().decode(errors="replace"))
                return 1
            time.sleep(0.1)
        else:
            print("локальный UI listener не запустился")
            return 1

        checks = []
        checks.append(("UI health доступен только на локальном порту", request(f"http://127.0.0.1:{ui_port}/health")[0] == 200))
        unauth, _ = request(f"http://127.0.0.1:{sync_port}/sync/hello")
        checks.append(("sync-only listener отклоняет запрос без token", unauth == 401))
        status, body = request(f"http://127.0.0.1:{sync_port}/sync/hello", TOKEN)
        hello = json.loads(body) if status == 200 else {}
        checks.append(("sync hello работает с token", status == 200 and hello.get("ok") is True))
        api_status, _ = request(f"http://127.0.0.1:{sync_port}/api/trpc/ping", TOKEN)
        checks.append(("sync-only listener не публикует пользовательский API", api_status == 404))
        blob_status, _ = request(f"http://127.0.0.1:{sync_port}/sync/blob/{'0' * 64}", TOKEN)
        checks.append(("CAS route существует, но неизвестный hash не выдаётся", blob_status in (401, 404)))
        failed = False
        for label, ok in checks:
            print(f"[{'OK  ' if ok else 'FAIL'}] {label}")
            failed |= not ok
        return 1 if failed else 0
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
        for suffix in ("", "-wal", "-shm"):
            try:
                Path(str(db) + suffix).unlink()
            except FileNotFoundError:
                pass


if __name__ == "__main__":
    raise SystemExit(main())
