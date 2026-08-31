"""Legacy relay E2E is intentionally retired.

It depended on client-supplied user IDs and sync-v1. New integration tests must use
HttpOnly session cookies against a local disposable database.
"""

raise SystemExit("Legacy relay E2E is disabled; use the local session smoke tests.")
