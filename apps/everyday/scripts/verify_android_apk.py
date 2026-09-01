"""Verify that a built APK contains the autonomous Rust node for every supported ABI."""

import argparse
import subprocess
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPOSITORY = ROOT.parent.parent
DEFAULT_APK = ROOT / "android/app/build/outputs/apk/debug/app-debug.apk"
ABIS = ("arm64-v8a", "x86_64")
JNI_SYMBOLS = (
    "Java_ru_meshkeeper_app_RustNode_startNode",
    "Java_ru_meshkeeper_app_RustNode_provisionNodeKey",
    "Java_ru_meshkeeper_app_RustNode_updateAdvertiseUrl",
    "Java_ru_meshkeeper_app_RustNode_fragmentTransport",
    "Java_ru_meshkeeper_app_RustNode_validateTransportFrame",
    "Java_ru_meshkeeper_app_RustNode_missingTransportRanges",
    "Java_ru_meshkeeper_app_RustNode_assembleTransport",
)


def source_revision() -> bytes:
    revision = subprocess.run(
        ["git", "rev-parse", "--verify", "HEAD"], cwd=REPOSITORY,
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    dirty = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=no"], cwd=REPOSITORY,
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    return f"{revision}{'-dirty' if dirty else ''}".encode()


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
        web_scripts = b"".join(
            archive.read(name) for name in names
            if name.startswith("assets/www/assets/") and name.endswith(".js")
        )
        if b"sendSyncBundleOverBle" not in web_scripts:
            raise SystemExit("APK web application does not expose the BLE transfer UI")
        if b"disableBleTransport" not in web_scripts:
            raise SystemExit("APK web application cannot stop the foreground BLE transport")
        if b"sync.reportTransportStatus" not in web_scripts:
            raise SystemExit("APK web application does not persist BLE transport diagnostics")
        if b"acknowledgePendingSyncBundle" not in web_scripts:
            raise SystemExit("APK web application does not acknowledge durable BLE imports")
        revision = source_revision()
        for abi in ABIS:
            member = f"lib/{abi}/libmeshkeeper_node.so"
            if member not in names:
                raise SystemExit(f"APK does not contain {member}")
            output = Path(temp) / f"{abi}.so"
            library = archive.read(member)
            output.write_bytes(library)
            if revision not in library:
                raise SystemExit(f"Source revision is missing from {member}")
            symbols = subprocess.run(
                ["readelf", "-Ws", output], check=True, capture_output=True, text=True
            ).stdout
            for symbol in JNI_SYMBOLS:
                if symbol not in symbols:
                    raise SystemExit(f"JNI entry point {symbol} is missing from {member}")

    print(f"Android APK verified: offline UI and Rust JNI node for {', '.join(ABIS)} ({apk.stat().st_size} bytes).")


if __name__ == "__main__":
    main()
