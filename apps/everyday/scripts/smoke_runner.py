"""Сквозная проверка узла MeshKeeper на чистой базе.

Поднимает release-бинарник на свободном порту с временной базой, прогоняет
сценарии ТЗ через HTTP (регистрация, каталог, выдача/возврат, приглашения,
роли, неисправности, инвентаризация, отчёты, CSRF, SPA-маршруты) и гасит узел.

    python scripts/smoke_runner.py

Требуется собранный узел и фронтенд: npm run build
"""

from __future__ import annotations

import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "backend" / "target" / "release" / (
    "meshkeeper-node.exe" if os.name == "nt" else "meshkeeper-node"
)
FALLBACK_BINARY = ROOT / "dist" / "server" / BINARY.name


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_ready(base: str, timeout: float = 25.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{base}/health", timeout=2):
                return True
        except (urllib.error.URLError, OSError):
            time.sleep(0.25)
    return False


def main() -> int:
    binary = BINARY if BINARY.is_file() else FALLBACK_BINARY
    if not binary.is_file():
        print(f"Узел не собран: нет {binary}. Выполните npm run build", file=sys.stderr)
        return 2

    port = free_port()
    base = f"http://127.0.0.1:{port}"
    db = Path(tempfile.gettempdir()) / f"meshkeeper-smoke-{uuid.uuid4().hex}.db"
    env = {
        **os.environ,
        "MESHKEEPER_DB": str(db),
        "MESHKEEPER_BIND": f"127.0.0.1:{port}",
        # Демо-данные и вход по списку не должны влиять на результат.
        "MESHKEEPER_DEMO_DATA": "0",
        "MESHKEEPER_DEMO_LOGIN": "0",
        "MESHKEEPER_OPEN_REGISTRATION": "0",
    }
    node = subprocess.Popen(
        [str(binary)],
        cwd=str(ROOT),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        if not wait_ready(base):
            print("Узел не поднялся", file=sys.stderr)
            return 1
        result = subprocess.run(
            [sys.executable, str(Path(__file__).with_name("smoke.py"))],
            cwd=str(ROOT),
            env={**env, "MK_BASE": base, "PYTHONIOENCODING": "utf-8"},
        )
        return result.returncode
    finally:
        node.terminate()
        try:
            node.wait(timeout=10)
        except subprocess.TimeoutExpired:
            node.kill()
        for suffix in ("", "-wal", "-shm"):
            Path(str(db) + suffix).unlink(missing_ok=True)


if __name__ == "__main__":
    raise SystemExit(main())
