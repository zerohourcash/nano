import os
from pathlib import Path
from urllib.parse import urlparse

import paramiko


def required_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise SystemExit(f"Required environment variable is not set: {name}")
    return value


def require_http_base() -> str:
    value = required_env("MESHKEEPER_BASE_URL").rstrip("/")
    parsed = urlparse(value)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        raise SystemExit("MESHKEEPER_BASE_URL must be an absolute HTTP(S) URL")
    loopback = parsed.hostname in {"localhost", "127.0.0.1", "::1"}
    if parsed.scheme != "https" and not loopback:
        raise SystemExit("MESHKEEPER_BASE_URL must use HTTPS outside loopback")
    return value


def connect_ssh() -> paramiko.SSHClient:
    host = required_env("MESHKEEPER_SSH_HOST")
    user = required_env("MESHKEEPER_SSH_USER")
    key_path = os.environ.get("MESHKEEPER_SSH_KEY", "").strip()
    known_hosts = os.environ.get("MESHKEEPER_KNOWN_HOSTS", "").strip()

    client = paramiko.SSHClient()
    client.load_system_host_keys()
    if known_hosts:
        path = Path(known_hosts).expanduser().resolve(strict=True)
        client.load_host_keys(str(path))
    client.set_missing_host_key_policy(paramiko.RejectPolicy())

    options = {
        "hostname": host,
        "username": user,
        "port": int(os.environ.get("MESHKEEPER_SSH_PORT", "22")),
        "timeout": 25,
        "banner_timeout": 25,
        "auth_timeout": 25,
        "allow_agent": True,
        "look_for_keys": True,
    }
    if key_path:
        options["key_filename"] = str(Path(key_path).expanduser().resolve(strict=True))
    client.connect(**options)
    return client
