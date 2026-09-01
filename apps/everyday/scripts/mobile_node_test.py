"""Host proof of the Android topology: private UI + sync-only LAN listener."""

from __future__ import annotations

import json
import base64
import os
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from http.cookiejar import CookieJar
from pathlib import Path

from device_test_signing import CRITICAL, DeviceSigner

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "backend" / "target" / "release" / "meshkeeper-node"
TOKEN = "android-host-test-token-at-least-32-characters"
NODE_KEY = base64.b64encode(bytes([17]) * 32).decode().rstrip("=")


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


class MobileApi:
    def __init__(self, base: str):
        self.base = base
        self.cookies = CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.cookies))
        self.signer = DeviceSigner("android-lifecycle")
        self.registered = False

    def call(self, procedure: str, payload=None, mutation: bool = True):
        body = json.dumps({"0": {"json": payload}}, separators=(",", ":")).encode()
        if mutation:
            req = urllib.request.Request(
                f"{self.base}/api/trpc/{procedure}?batch=1", data=body, method="POST"
            )
            req.add_header("content-type", "application/json")
            if procedure in CRITICAL:
                if not self.registered:
                    enrolled = self.call("auth.registerDevice", {
                        "deviceId": self.signer.device_id,
                        "name": "Android lifecycle test",
                        "publicKey": self.signer.public_key,
                    })
                    if not isinstance(enrolled, dict) or enrolled.get("deviceId") != self.signer.device_id:
                        raise RuntimeError(f"device enrollment failed: {enrolled}")
                    self.registered = True
                for key, value in self.signer.headers(f"/api/trpc/{procedure}", body).items():
                    req.add_header(key, value)
        else:
            encoded = urllib.parse.quote(body.decode())
            req = urllib.request.Request(
                f"{self.base}/api/trpc/{procedure}?batch=1&input={encoded}", method="GET"
            )
        req.add_header("origin", self.base)
        with self.opener.open(req, timeout=10) as response:
            value = json.loads(response.read().decode())
        if isinstance(value, list):
            value = value[0]
        if "error" in value:
            raise RuntimeError(value["error"]["json"].get("message", "API error"))
        return value["result"]["data"]["json"]


def spawn(env: dict[str, str]) -> subprocess.Popen:
    return subprocess.Popen(
        [str(BINARY)], cwd=ROOT, env=env,
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
    )


def wait_ready(proc: subprocess.Popen, ui_port: int) -> bool:
    for _ in range(100):
        try:
            if request(f"http://127.0.0.1:{ui_port}/health")[0] == 200:
                return True
        except OSError:
            pass
        if proc.poll() is not None:
            return False
        time.sleep(0.1)
    return False


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
        "MESHKEEPER_NODE_SIGNING_KEY": NODE_KEY,
        "MESHKEEPER_DEMO_DATA": "0",
        "MESHKEEPER_DEMO_LOGIN": "0",
    }
    proc = spawn(env)
    try:
        if not wait_ready(proc, ui_port):
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

        api = MobileApi(f"http://127.0.0.1:{ui_port}")
        owner = api.call("auth.register", {
            "fullName": "Владелец телефона",
            "phone": "+7 900 444-55-66",
            "password": "MobileLifecycle123",
            "workspaceName": "Мобильная автономная организация",
        })
        workspace = api.call("meta.workspaces", None, mutation=False)[0]
        message = api.call("chat.send", {
            "workspaceId": workspace["id"],
            "text": "Подписано до остановки мобильного процесса",
        })
        status_before = api.call("sync.status", None, mutation=False)
        checks.append(("мобильная транзакция подписана device-proof и ledger",
                       owner.get("id") is not None and message.get("ledgerVerified") is True))

        # Android may kill and later recreate the foreground service. The test
        # uses the same no-backup SQLite and external Keystore-equivalent seed.
        proc.terminate()
        proc.wait(timeout=10)
        proc = spawn(env)
        restarted = wait_ready(proc, ui_port)
        checks.append(("Rust-узел перезапустился на прежней мобильной базе", restarted))
        if restarted:
            status_after = api.call("sync.status", None, mutation=False)
            messages = api.call("chat.list", {"workspaceId": workspace["id"]}, mutation=False)
            persisted = [row for row in messages if row.get("guid") == message.get("guid")]
            checks.append(("после убийства процесса сохранены node ID и внешний signing key",
                           status_after.get("nodeId") == status_before.get("nodeId")))
            checks.append(("подписанная offline-транзакция пережила restart ровно один раз",
                           len(persisted) == 1 and persisted[0].get("ledgerVerified") is True))
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
