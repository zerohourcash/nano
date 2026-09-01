"""Verify that a built APK contains the autonomous Rust node for every supported ABI."""

import argparse
import subprocess
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_APK = ROOT / "android/app/build/outputs/apk/debug/app-debug.apk"
ABIS = ("arm64-v8a", "x86_64")
JNI_SYMBOLS = (
    "Java_ru_meshkeeper_app_RustNode_startNode",
    "Java_ru_meshkeeper_app_RustNode_provisionNodeKey",
    "Java_ru_meshkeeper_app_RustNode_updateAdvertiseUrl",
)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("apk", nargs="?", type=Path, default=DEFAULT_APK)
    args = parser.parse_args()
    apk = args.apk.resolve()
    if not apk.is_file():
        raise SystemExit(f"APK not found: {apk}")

    with zipfile.ZipFile(apk) as archive, tempfile.TemporaryDirectory() as temp:
        names = set(archive.namelist())
        if "assets/www/index.html" not in names:
            raise SystemExit("APK does not contain the offline web application")
        for abi in ABIS:
            member = f"lib/{abi}/libmeshkeeper_node.so"
            if member not in names:
                raise SystemExit(f"APK does not contain {member}")
            output = Path(temp) / f"{abi}.so"
            output.write_bytes(archive.read(member))
            symbols = subprocess.run(
                ["readelf", "-Ws", output], check=True, capture_output=True, text=True
            ).stdout
            for symbol in JNI_SYMBOLS:
                if symbol not in symbols:
                    raise SystemExit(f"JNI entry point {symbol} is missing from {member}")

    print(f"Android APK verified: offline UI and Rust JNI node for {', '.join(ABIS)} ({apk.stat().st_size} bytes).")


if __name__ == "__main__":
    main()
