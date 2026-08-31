"""Минимальный gate против опасного расхождения transport-документации и кода."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
transport = (ROOT / "docs" / "TRANSPORT.md").read_text(encoding="utf-8")
main = (ROOT / "apps" / "everyday" / "backend" / "src" / "main.rs").read_text(encoding="utf-8")

required_docs = {
    "/sync/journal": "journal endpoint",
    "/sync/journal/pull": "frontier endpoint",
    "/sync/blob/{hash}": "CAS chunk endpoint",
    "everyday-sync-bundle": "offline bundle",
    "SHA-256": "implemented digest",
    "Ed25519": "implemented signature",
    "ещё не реализованы": "honest mobile/BLE status",
}
for marker, label in required_docs.items():
    if marker not in transport:
        raise SystemExit(f"TRANSPORT.md: отсутствует {label}: {marker}")

for obsolete in ("BLAKE2b-256", "/api/events", 'protocol: "nano-inventory/1"'):
    if obsolete in transport:
        raise SystemExit(f"TRANSPORT.md: найден устаревший контракт: {obsolete}")

required_routes = (
    '"/sync/journal"',
    '"/sync/journal/pull"',
    '"/sync/blob/{hash}"',
)
for route in required_routes:
    if route not in main:
        raise SystemExit(f"main.rs больше не содержит документированный route {route}")

print("Transport documentation check passed: protocol markers match production routes.")
