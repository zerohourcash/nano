"""Static contract gate for the autonomous Android Rust-node packaging."""

from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
android = ROOT / "android" / "app"
lib = (ROOT / "backend" / "src" / "lib.rs").read_text(encoding="utf-8")
service = (android / "src/main/java/ru/meshkeeper/app/NodeService.java").read_text(encoding="utf-8")
activity = (android / "src/main/java/ru/meshkeeper/app/MainActivity.java").read_text(encoding="utf-8")
secrets = (android / "src/main/java/ru/meshkeeper/app/SecretStore.java").read_text(encoding="utf-8")
gradle = (android / "build.gradle").read_text(encoding="utf-8")
manifest = (android / "src/main/AndroidManifest.xml").read_text(encoding="utf-8")
layout = (android / "src/main/res/layout/activity_main.xml").read_text(encoding="utf-8")

required = {
    "Rust JNI symbol": "Java_ru_meshkeeper_app_RustNode_startNode" in lib,
    "node key migration JNI": "Java_ru_meshkeeper_app_RustNode_provisionNodeKey" in lib,
    "private UI bind": 'MESHKEEPER_BIND", "127.0.0.1:8765' in lib,
    "sync-only LAN bind": 'MESHKEEPER_SYNC_BIND", "0.0.0.0:8766' in lib,
    "foreground Rust launch": "RustNode.startNode" in service,
    "localhost WebView": "RustNode.localOrigin()" in activity,
    "cargo-ndk build": "buildRustNode" in gradle and "--lib" in gradle,
    "two supported ABIs": "arm64-v8a" in gradle and "x86_64" in gradle,
    "Android Keystore": 'KEYSTORE = "AndroidKeyStore"' in secrets,
    "authenticated token encryption": 'AES/GCM/NoPadding' in secrets and "updateAAD" in secrets,
    "legacy plaintext migration": "saveSyncToken(context, legacy)" in secrets and "editor.remove(legacyName)" in secrets,
    "service decrypts token": "SecretStore.loadSyncToken(this)" in service,
    "service seals node key": "SecretStore.saveNodeSigningKey(this" in service,
    "Rust receives sealed node key": "MESHKEEPER_NODE_SIGNING_KEY" in lib,
    "Rust receives organization scope": "MESHKEEPER_SYNC_WORKSPACES" in lib and "workspaceScope" in service,
    "native organization scope control": '@+id/workspaceScope' in layout and "normalizeWorkspaceScope" in activity,
    "authenticated LAN discovery": "MESHKEEPER_DISCOVERY_BIND" in lib and "discovery::run" in lib,
    "token absent from service Intent": "EXTRA_TOKEN" not in service and "EXTRA_TOKEN" not in activity,
    "scope absent from service Intent": "EXTRA_WORKSPACE_SCOPE" not in service and "EXTRA_WORKSPACE_SCOPE" not in activity,
    "token not restored into UI": "syncToken.setText(SecretStore.loadSyncToken" not in activity,
    "system Share receive": "android.intent.action.SEND" in manifest and "takePendingSyncBundle" in activity,
    "incoming bundle bounded": "MAX_SYNC_BUNDLE_BYTES" in activity and "content\".equalsIgnoreCase" in activity,
}
missing = [name for name, present in required.items() if not present]
if missing:
    raise SystemExit("Android Rust-node contract incomplete: " + ", ".join(missing))

legacy = android / "src/main/java/ru/meshkeeper/app/node"
if legacy.exists() and any(legacy.glob("*.java")):
    raise SystemExit("Обнаружена запрещённая дублирующая Java-реализация backend")

print("Android node check passed: JNI, isolated listeners, Keystore, cargo-ndk and no Java backend duplicate.")
