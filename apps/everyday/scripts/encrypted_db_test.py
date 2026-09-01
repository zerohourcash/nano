"""Build and prove the fail-closed SQLCipher production profile."""

from __future__ import annotations

import os
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path(tempfile.gettempdir()) / "meshkeeper-sqlcipher-target"
BINARY = TARGET / "release" / ("meshkeeper-node.exe" if os.name == "nt" else "meshkeeper-node")
PLAIN_BINARY = ROOT / "backend/target/release" / ("meshkeeper-node.exe" if os.name == "nt" else "meshkeeper-node")
GOOD_KEY = "correct-production-database-key-32-bytes"
WRONG_KEY = "wrong-production-database-key-00000"


def check(label: str, condition: bool) -> None:
    print(f"[{'OK  ' if condition else 'FAIL'}] {label}")
    if not condition:
        raise RuntimeError(label)


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def environment(db: Path, port: int, key: str | None) -> dict[str, str]:
    env = {
        **os.environ,
        "MESHKEEPER_DB": str(db),
        "MESHKEEPER_BIND": f"127.0.0.1:{port}",
        "MESHKEEPER_DEMO_DATA": "0",
        "MESHKEEPER_DEMO_LOGIN": "0",
    }
    env.pop("MESHKEEPER_DB_KEY", None)
    if key is not None:
        env["MESHKEEPER_DB_KEY"] = key
    return env


def stopped(db: Path, key: str | None, binary: Path = BINARY) -> tuple[int, str]:
    result = subprocess.run(
        [str(binary)], cwd=ROOT, env=environment(db, free_port(), key),
        capture_output=True, text=True, timeout=15,
    )
    return result.returncode, result.stderr


def start(db: Path, key: str) -> tuple[subprocess.Popen, int]:
    port = free_port()
    process = subprocess.Popen(
        [str(BINARY)], cwd=ROOT, env=environment(db, port, key),
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
    )
    for _ in range(100):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as response:
                if response.status == 200:
                    return process, port
        except OSError:
            pass
        if process.poll() is not None:
            break
        time.sleep(0.1)
    error = process.stderr.read().decode(errors="replace") if process.stderr else ""
    raise RuntimeError(f"SQLCipher node did not start: {error}")


def main() -> int:
    subprocess.run([
        "cargo", "build", "--release", "--features", "encrypted-db",
        "--target-dir", str(TARGET), "--manifest-path", str(ROOT / "backend/Cargo.toml"),
    ], cwd=ROOT, check=True)
    subprocess.run([
        "cargo", "build", "--release", "--manifest-path", str(ROOT / "backend/Cargo.toml"),
    ], cwd=ROOT, check=True)
    db_dir = Path(tempfile.mkdtemp(prefix="meshkeeper-sqlcipher-test-"))
    db = db_dir / "encrypted.db"
    try:
        code, error = stopped(db_dir / "must-not-open.db", GOOD_KEY, PLAIN_BINARY)
        check("plain build fails closed when an encryption key is configured",
              code != 0 and "без encrypted-db" in error)
        code, error = stopped(db, None)
        check("SQLCipher build refuses to start without a key", code != 0 and "MESHKEEPER_DB_KEY" in error)
        code, error = stopped(db, "too-short")
        check("SQLCipher build rejects a short key", code != 0 and "слишком короткий" in error)

        process, _port = start(db, GOOD_KEY)
        process.terminate()
        process.wait(timeout=10)
        check("encrypted database was created", db.is_file() and db.stat().st_size > 0)
        check("database has no plaintext SQLite header", db.read_bytes()[:16] != b"SQLite format 3\x00")

        code, error = stopped(db, WRONG_KEY)
        check("wrong key cannot open the database", code != 0 and "повреждена" in error)
        process, _port = start(db, GOOD_KEY)
        process.terminate()
        process.wait(timeout=10)
        check("correct key reopens the existing database", process.returncode == 0 or process.returncode == -15)
    finally:
        shutil.rmtree(db_dir, ignore_errors=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
