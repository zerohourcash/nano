"""Fail closed when backend, browser and HTTP test signature policies drift."""

from __future__ import annotations

import ast
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Authentication/capability establishment cannot use an already enrolled device.
# Notification acknowledgement is private UI state, and inventory.verifyAct only
# validates supplied bytes despite using POST for its potentially large payload.
EXPLICIT_UNSIGNED_MUTATIONS = {
    "auth.join",
    "auth.joinRegister",
    "auth.login",
    "auth.logout",
    "auth.register",
    "auth.registerDevice",
    "inventory.verifyAct",
    "notifications.markAllRead",
    "notifications.markRead",
}


def strings_between(path: str, start_marker: str, end_marker: str) -> set[str]:
    source = (ROOT / path).read_text(encoding="utf-8")
    start = source.index(start_marker)
    end = source.index(end_marker, start + len(start_marker))
    return set(re.findall(r'[\"\']([A-Za-z][A-Za-z0-9_.]+)[\"\']', source[start:end]))


def python_critical() -> set[str]:
    tree = ast.parse((ROOT / "scripts/device_test_signing.py").read_text(encoding="utf-8"))
    for node in ast.walk(tree):
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id == "CRITICAL" for target in node.targets
        ):
            return {
                element.value
                for element in node.value.elts
                if isinstance(element, ast.Constant)
            }
    raise AssertionError("scripts/device_test_signing.py has no CRITICAL set")


def dispatch_procedures() -> set[str]:
    source = (ROOT / "backend/src/api.rs").read_text(encoding="utf-8")
    start = source.index("fn dispatch_inner")
    end = source.index("\nfn auth_directory", start)
    return set(
        re.findall(r'^\s*[\"\']([A-Za-z][A-Za-z0-9_.]+)[\"\']\s*=>', source[start:end], re.MULTILINE)
    )


def fail(label: str, values: set[str]) -> None:
    if values:
        raise SystemExit(f"{label}: {', '.join(sorted(values))}")


rust = strings_between(
    "backend/src/device.rs", "pub fn requires_signature", "\npub fn register"
)
browser = strings_between(
    "src/lib/device-signing.ts", "const CRITICAL = [", "\n]\n"
)
python = python_critical()
dispatch = dispatch_procedures()
read_only = strings_between("backend/src/api.rs", "pub fn is_mutation", "\nfn g")

fail("browser signature policy missing backend procedures", rust - browser)
fail("browser signs procedures absent from backend policy", browser - rust)
fail("HTTP test signer missing backend procedures", rust - python)
fail("HTTP test signer has procedures absent from backend policy", python - rust)
fail("signature policy references unknown dispatch procedures", rust - dispatch)

unsigned_mutations = dispatch - read_only - rust
fail(
    "new mutation has no signature policy decision",
    unsigned_mutations - EXPLICIT_UNSIGNED_MUTATIONS,
)
fail(
    "stale explicit unsigned mutation allowlist",
    EXPLICIT_UNSIGNED_MUTATIONS - unsigned_mutations,
)

print(
    f"Signature policy contract passed: {len(rust)} signed, "
    f"{len(EXPLICIT_UNSIGNED_MUTATIONS)} explicitly unsigned mutations."
)
