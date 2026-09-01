#!/usr/bin/env python3
"""Fail-closed HTTP distribution test for an operator-provided Android APK."""

from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import tempfile
import time
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "backend/target/release/meshkeeper-node"


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_health(base: str, process: subprocess.Popen[bytes]) -> None:
    for _ in range(100):
        if process.poll() is not None:
            raise RuntimeError("node exited before health became available")
        try:
            with urllib.request.urlopen(f"{base}/health", timeout=1) as response:
                if response.status == 200:
                    return
        except OSError:
            time.sleep(0.05)
    raise RuntimeError("node did not become healthy")


def main() -> None:
    payload = b"Everyday Android test package\x00" + bytes(range(256)) * 32
    expected = hashlib.sha256(payload).hexdigest()
    with tempfile.TemporaryDirectory(prefix="everyday-apk-http-") as temporary:
        temp = Path(temporary)
        apk = temp / "everyday-test.apk"
        apk.write_bytes(payload)
        desktop_payload = b"Everyday desktop archive\x00" + bytes(reversed(range(256))) * 16
        desktop = temp / "everyday-linux.tar.gz"
        desktop.write_bytes(desktop_payload)
        desktop_expected = hashlib.sha256(desktop_payload).hexdigest()
        port = free_port()
        env = os.environ.copy()
        env.update(
            {
                "MESHKEEPER_BIND": f"127.0.0.1:{port}",
                "MESHKEEPER_DB": str(temp / "node.db"),
                "MESHKEEPER_NO_SEED": "1",
                "MESHKEEPER_WEB_ROOT": str(ROOT / "dist/public"),
                "MESHKEEPER_ANDROID_APK_PATH": str(apk),
                "MESHKEEPER_ANDROID_APK_SHA256": expected,
                "MESHKEEPER_DESKTOP_RELEASE_PATH": str(desktop),
                "MESHKEEPER_DESKTOP_RELEASE_SHA256": desktop_expected,
            }
        )
        process = subprocess.Popen(
            [str(BINARY)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        )
        base = f"http://127.0.0.1:{port}"
        try:
            wait_health(base, process)
            options_url = (
                f"{base}/api/trpc/auth.options?input=%7B%22json%22%3Anull%7D"
            )
            with urllib.request.urlopen(options_url, timeout=3) as response:
                options = json.load(response)["result"]["data"]["json"]
            release = options["androidTestBuild"]
            assert release["sha256"] == expected
            assert release["sizeBytes"] == len(payload)
            assert release["debug"] is True
            with urllib.request.urlopen(base + release["url"], timeout=3) as response:
                downloaded = response.read()
                assert response.status == 200
                assert response.headers.get_content_type() in {
                    "application/vnd.android.package-archive",
                    "application/octet-stream",
                }
            assert downloaded == payload
            desktop_release = options["desktopTestBuild"]
            assert desktop_release["sha256"] == desktop_expected
            assert desktop_release["sizeBytes"] == len(desktop_payload)
            assert desktop_release["debug"] is False
            with urllib.request.urlopen(base + desktop_release["url"], timeout=3) as response:
                assert response.status == 200
                assert response.read() == desktop_payload
            print("[OK  ] release metadata and downloaded Android/desktop bytes match SHA-256")
        finally:
            process.terminate()
            process.wait(timeout=5)

        bad_env = env | {
            "MESHKEEPER_BIND": f"127.0.0.1:{free_port()}",
            "MESHKEEPER_DB": str(temp / "rejected.db"),
            "MESHKEEPER_ANDROID_APK_SHA256": "0" * 64,
        }
        rejected = subprocess.run(
            [str(BINARY)], env=bad_env, capture_output=True, timeout=10, check=False
        )
        assert rejected.returncode != 0
        assert b"SHA-256 mismatch" in rejected.stderr
        assert not (temp / "rejected.db").exists(), "bad release must fail before DB startup"
        print("[OK  ] mismatched APK hash prevents node startup before database creation")

        bad_desktop_env = env | {
            "MESHKEEPER_BIND": f"127.0.0.1:{free_port()}",
            "MESHKEEPER_DB": str(temp / "desktop-rejected.db"),
            "MESHKEEPER_DESKTOP_RELEASE_SHA256": "f" * 64,
        }
        desktop_rejected = subprocess.run(
            [str(BINARY)], env=bad_desktop_env, capture_output=True, timeout=10, check=False
        )
        assert desktop_rejected.returncode != 0
        assert b"Desktop release SHA-256 mismatch" in desktop_rejected.stderr
        assert not (temp / "desktop-rejected.db").exists()
        print("[OK  ] mismatched desktop hash prevents node startup before database creation")


if __name__ == "__main__":
    main()
