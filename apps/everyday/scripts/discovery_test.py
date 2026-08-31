"""Real UDP test for authenticated, serverless LAN peer discovery."""

from __future__ import annotations

import socket
import sqlite3
import time
import json
import urllib.request

from sync_test import OWNER_PASSWORD, OWNER_PHONE, TOKEN, Node, check, failures, free_port, wait_for


def udp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def peers(node: Node) -> list[str]:
    with sqlite3.connect(node.db) as database:
        return [row[0] for row in database.execute("SELECT url FROM peers ORDER BY url")]


def journal(node: Node) -> dict:
    request = urllib.request.Request(f"{node.base}/sync/journal")
    request.add_header("authorization", f"Bearer {TOKEN}")
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.loads(response.read().decode())


def main() -> int:
    failures.clear()
    a_http, b_http, attacker_http = free_port(), free_port(), free_port()
    a_udp, b_udp, attacker_udp = udp_port(), udp_port(), udp_port()
    common = {
        "MESHKEEPER_SYNC_TOKEN": TOKEN,
        "MESHKEEPER_SYNC_INTERVAL": "5",
        "MESHKEEPER_ALLOW_INSECURE_SYNC": "1",
    }
    a = Node("discover-a", a_http, {
        **common,
        "MESHKEEPER_ADVERTISE_URL": f"http://127.0.0.1:{a_http}",
        "MESHKEEPER_DISCOVERY_BIND": f"127.0.0.1:{a_udp}",
        "MESHKEEPER_DISCOVERY_TARGET": f"127.0.0.1:{b_udp}",
    })
    b = Node("discover-b", b_http, {
        **common,
        "MESHKEEPER_ADVERTISE_URL": f"http://127.0.0.1:{b_http}",
        "MESHKEEPER_DISCOVERY_BIND": f"127.0.0.1:{b_udp}",
        "MESHKEEPER_DISCOVERY_TARGET": f"127.0.0.1:{a_udp}",
    })
    attacker = Node("discover-wrong-token", attacker_http, {
        **common,
        "MESHKEEPER_SYNC_TOKEN": "different-discovery-token-at-least-32-chars",
        "MESHKEEPER_ADVERTISE_URL": f"http://127.0.0.1:{attacker_http}",
        "MESHKEEPER_DISCOVERY_BIND": f"127.0.0.1:{attacker_udp}",
        "MESHKEEPER_DISCOVERY_TARGET": f"127.0.0.1:{a_udp}",
    })
    try:
        check("три ноды discovery запущены", a.wait_ready() and b.wait_ready() and attacker.wait_ready())
        owner = a.call("auth.register", {
            "fullName": "Владелец Discovery",
            "phone": OWNER_PHONE,
            "password": OWNER_PASSWORD,
            "workspaceName": "Автономная LAN организация",
        })
        check("организация создана без заданных peers", isinstance(owner, dict) and "id" in owner)
        check("A и B нашли друг друга по HMAC UDP", wait_for(
            lambda: b.base in peers(a) and a.base in peers(b), timeout=20
        ), f"A={peers(a)} B={peers(b)}")
        check("журнал автоматически сошёлся после discovery", wait_for(
            lambda: len(journal(b).get("workspaces", [])) == 1, timeout=25
        ))
        login = b.call("auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD})
        check("после discovery владелец входит на B офлайн", isinstance(login, dict) and "id" in login)
        time.sleep(6)
        check("анонс с чужим токеном не добавлен", attacker.base not in peers(a), str(peers(a)))
    finally:
        a.stop()
        b.stop()
        attacker.stop()

    print("\n===== DISCOVERY ИТОГ =====")
    print("failed:", len(failures))
    for failure in failures:
        print(" -", failure)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
