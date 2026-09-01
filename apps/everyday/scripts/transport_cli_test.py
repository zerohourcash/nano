#!/usr/bin/env python3
"""Exercise the real MKST pipe/serial bridge as independent processes."""

import base64
import random
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CLI = ROOT / "backend/target/release/meshkeeper-frame"


def run(args: list[str], payload: bytes, ok: bool = True) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run([str(CLI), *args], input=payload, capture_output=True, check=False)
    if ok and result.returncode != 0:
        raise AssertionError(result.stderr.decode("utf-8", "replace"))
    if not ok and result.returncode == 0:
        raise AssertionError(f"command unexpectedly succeeded: {args}")
    return result


def passed(label: str) -> None:
    print(f"[OK  ] {label}")


def main() -> None:
    if not CLI.is_file():
        raise SystemExit(f"build the release binaries first: {CLI}")
    payload = bytes((index * 17 + 31) % 251 for index in range(96_000))
    fragmented = run(["fragment", "bundle", "128"], payload).stdout
    lines = fragmented.splitlines()
    assert len(lines) > 100
    passed(f"opaque bundle fragmented for constrained MTU frames={len(lines)}")

    random.Random(20260901).shuffle(lines)
    lines.extend((lines[3], lines[3]))
    transported = b"\n".join(lines) + b"\n"
    run(["verify"], transported)
    restored = run(["assemble"], transported).stdout
    assert restored == payload
    passed("reordered frames and identical radio retries reassemble byte-for-byte")

    missing = b"\n".join(lines[:-12]) + b"\n"
    rejected = run(["assemble"], missing, ok=False)
    assert b"missing ranges" in rejected.stderr
    passed("frame loss fails closed and reports retransmission ranges")

    padding = b"=" * (-len(lines[0]) % 4)
    damaged = bytearray(base64.urlsafe_b64decode(lines[0] + padding))
    damaged[-1] ^= 1
    forged = base64.urlsafe_b64encode(damaged).rstrip(b"=")
    corrupted = b"\n".join([forged, *lines[1:]]) + b"\n"
    rejected = run(["verify"], corrupted, ok=False)
    assert b"checksum" in rejected.stderr
    passed("per-frame corruption is rejected before assembly")

    run(["fragment", "bundle", "76"], b"too-small-mtu", ok=False)
    run(["fragment", "unknown", "128"], b"unknown-kind", ok=False)
    passed("invalid MTU and payload kinds are rejected")
    print("\n===== TRANSPORT CLI ИТОГ =====\nfailed: 0")


if __name__ == "__main__":
    main()
