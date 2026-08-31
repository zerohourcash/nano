"""Static contract gate for the autonomous Android Rust-node packaging."""

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
android = ROOT / "android" / "app"
lib = (ROOT / "backend" / "src" / "lib.rs").read_text(encoding="utf-8")
service = (android / "src/main/java/ru/meshkeeper/app/NodeService.java").read_text(encoding="utf-8")
activity = (android / "src/main/java/ru/meshkeeper/app/MainActivity.java").read_text(encoding="utf-8")
gradle = (android / "build.gradle").read_text(encoding="utf-8")

required = {
    "Rust JNI symbol": "Java_ru_meshkeeper_app_RustNode_startNode" in lib,
    "private UI bind": 'MESHKEEPER_BIND", "127.0.0.1:8765' in lib,
    "sync-only LAN bind": 'MESHKEEPER_SYNC_BIND", "0.0.0.0:8766' in lib,
    "foreground Rust launch": "RustNode.startNode" in service,
    "localhost WebView": "RustNode.localOrigin()" in activity,
    "cargo-ndk build": "buildRustNode" in gradle and "--lib" in gradle,
    "two supported ABIs": "arm64-v8a" in gradle and "x86_64" in gradle,
}
missing = [name for name, present in required.items() if not present]
if missing:
    raise SystemExit("Android Rust-node contract incomplete: " + ", ".join(missing))

legacy = android / "src/main/java/ru/meshkeeper/app/node"
if legacy.exists() and any(legacy.glob("*.java")):
    raise SystemExit("Обнаружена запрещённая дублирующая Java-реализация backend")

print("Android node check passed: JNI, isolated listeners, cargo-ndk and no Java backend duplicate.")
