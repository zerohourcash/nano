"""Retired legacy relay deployment entrypoint.

The old script published authenticated traffic over plain HTTP and deployed a
sync-v1 node that could read the entire journal. Keeping a fail-closed command is
safer than silently recreating that exposure. Deploy sync-v2 behind HTTPS using
infrastructure reviewed for the target environment.
"""

raise SystemExit(
    "Legacy relay deployment is disabled: it is not safe for production. "
    "Use an HTTPS-only sync-v2 deployment after protocol migration."
)
