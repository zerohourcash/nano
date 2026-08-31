"""Масштабный тест 100 реальных процессов Everyday в разреженной mesh-сети."""

from __future__ import annotations

import json
import base64
import hashlib
import os
import socket
import sqlite3
import statistics
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor

from sync_test import OWNER_PASSWORD, OWNER_PHONE, TOKEN, Node, check, failures


def reserve_ports(count: int) -> list[int]:
    sockets: list[socket.socket] = []
    try:
        for _ in range(count):
            sock = socket.socket()
            sock.bind(("127.0.0.1", 0))
            sockets.append(sock)
        return [sock.getsockname()[1] for sock in sockets]
    finally:
        for sock in sockets:
            sock.close()


def journal(node: Node) -> dict:
    request = urllib.request.Request(f"{node.base}/sync/journal")
    request.add_header("authorization", f"Bearer {TOKEN}")
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.loads(response.read().decode())


def parallel_map(nodes: list[Node], fn):
    with ThreadPoolExecutor(max_workers=min(32, len(nodes))) as pool:
        return list(pool.map(fn, nodes))


def wait_count(nodes: list[Node], predicate, timeout: float, label: str) -> bool:
    deadline = time.monotonic() + timeout
    last = 0
    while time.monotonic() < deadline:
        def probe(node: Node) -> bool:
            try:
                return predicate(journal(node))
            except Exception:
                return False
        states = parallel_map(nodes, probe)
        last = sum(states)
        if last == len(nodes):
            return True
        time.sleep(2)
    print(f"[INFO] {label}: сошлись {last}/{len(nodes)}")
    return False


def rss_kib(node: Node) -> int:
    try:
        for line in open(f"/proc/{node.proc.pid}/status", encoding="utf-8"):
            if line.startswith("VmRSS:"):
                return int(line.split()[1])
    except OSError:
        pass
    return 0


def kv_number(node: Node, key: str) -> int:
    try:
        with sqlite3.connect(node.db, timeout=5) as conn:
            row = conn.execute("SELECT v FROM kv WHERE k=?", (key,)).fetchone()
        return int(row[0]) if row else 0
    except (sqlite3.Error, ValueError):
        return 0


def has_blob(node: Node, blob_hash: str) -> bool:
    try:
        with sqlite3.connect(node.db, timeout=5) as conn:
            return conn.execute(
                "SELECT COUNT(*) FROM content_blobs WHERE hash=?", (blob_hash,)
            ).fetchone()[0] == 1
    except sqlite3.Error:
        return False


def main() -> int:
    failures.clear()
    count = int(os.environ.get("MESHKEEPER_SCALE_NODES", "100"))
    if count < 10:
        raise SystemExit("MESHKEEPER_SCALE_NODES должен быть не меньше 10")
    ports = reserve_ports(count)
    bases = [f"http://127.0.0.1:{port}" for port in ports]
    nodes: list[Node] = []
    started = time.monotonic()
    try:
        for index, port in enumerate(ports):
            neighbor_indexes = {
                (index - 1) % count,
                (index + 1) % count,
                (index - 10) % count,
                (index + 10) % count,
            }
            peers = ",".join(bases[peer] for peer in sorted(neighbor_indexes) if peer != index)
            nodes.append(
                Node(
                    f"scale-{index:03d}",
                    port,
                    {
                        "MESHKEEPER_SYNC_TOKEN": TOKEN,
                        "MESHKEEPER_SYNC_INTERVAL": "5",
                        "MESHKEEPER_ALLOW_INSECURE_SYNC": "1",
                        "MESHKEEPER_ADVERTISE_URL": bases[index],
                        "MESHKEEPER_PEERS": peers,
                    },
                )
            )
        check(f"{count} процессов запущены", all(parallel_map(nodes, lambda node: node.wait_ready(40))))

        root = nodes[0]
        owner = root.call(
            "auth.register",
            {
                "fullName": "Владелец 100-node mesh",
                "phone": OWNER_PHONE,
                "password": OWNER_PASSWORD,
                "workspaceName": "Масштабная автономная организация",
            },
        )
        check("организация создана на корневом узле", isinstance(owner, dict) and "id" in owner)
        converged_at = time.monotonic()
        converged = wait_count(
            nodes,
            lambda data: len(data.get("workspaces", [])) == 1,
            timeout=120,
            label="первичная организация",
        )
        convergence_seconds = time.monotonic() - converged_at
        check(f"организация сошлась на {count}/{count} узлах", converged, f"{convergence_seconds:.1f} с")

        leaf = nodes[-1]
        logged = leaf.call("auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD})
        check("офлайн-вход работает на удалённом узле", isinstance(logged, dict) and "id" in logged)
        workspace_id = leaf.call("meta.workspaces", None, mutation=False)[0]["id"]
        message = leaf.call(
            "chat.send",
            {"workspaceId": workspace_id, "text": "Сообщение через сеть из 100 узлов"},
        )
        scale_photo_bytes = bytes([91]) * 70_000
        scale_photo_hash = hashlib.sha256(scale_photo_bytes).hexdigest()
        scale_photo = "data:image/webp;base64," + base64.b64encode(scale_photo_bytes).decode()
        item = leaf.call(
            "items.create",
            {
                "workspaceId": workspace_id,
                "title": "Рация масштабного теста",
                "photos": [{"url": scale_photo, "thumbUrl": scale_photo}],
            },
        )
        taken = leaf.call(
            "transfers.take",
            {"itemId": item.get("id"), "dueAt": "2026-09-30T12:00:00.000Z"},
        )
        check(
            "leaf создал подписанные чат и выдачу",
            message.get("ledgerVerified") is True
            and taken.get("status", {}).get("slug") == "in-work",
        )
        propagated_at = time.monotonic()
        propagated = wait_count(
            [root],
            lambda data: any(row.get("guid") == message.get("guid") for row in data.get("messages", []))
            and any(
                row.get("title") == "Рация масштабного теста" and row.get("statusSlug") == "in-work"
                for row in data.get("items", [])
            )
            and has_blob(root, scale_photo_hash),
            timeout=90,
            label="операции leaf→root",
        )
        propagation_seconds = time.monotonic() - propagated_at
        check("подписанные операции дошли leaf→root", propagated, f"{propagation_seconds:.1f} с")
        check("CAS-вложение дошло leaf→root и прошло SHA-256", has_blob(root, scale_photo_hash))

        failed_indexes = list(range(5, count, 10))
        failed_nodes = [nodes[index] for index in failed_indexes]
        for node in failed_nodes:
            node.stop(cleanup=False)
        second = leaf.call(
            "chat.send",
            {"workspaceId": workspace_id, "text": "Сеть работает после потери десяти узлов"},
        )
        survived = wait_count(
            [root],
            lambda data: any(row.get("guid") == second.get("guid") for row in data.get("messages", [])),
            timeout=90,
            label="работа после отказа",
        )
        check(f"сеть работает после отключения {len(failed_nodes)} узлов", survived)

        for node in failed_nodes:
            node.restart()
        check("отключённые процессы перезапущены", all(parallel_map(failed_nodes, lambda node: node.wait_ready(40))))
        recovered = wait_count(
            failed_nodes,
            lambda data: any(row.get("guid") == second.get("guid") for row in data.get("messages", [])),
            timeout=120,
            label="восстановленные узлы",
        )
        check(f"{len(failed_nodes)}/{len(failed_nodes)} восстановленных узлов догнали журнал", recovered)
        recovered_blobs = wait_count(
            failed_nodes,
            lambda _data: True,
            timeout=120,
            label="CAS восстановленных узлов",
        ) and all(parallel_map(failed_nodes, lambda node: has_blob(node, scale_photo_hash)))
        check("восстановленные узлы догнали CAS-вложение", recovered_blobs)

        memories = parallel_map(nodes, rss_kib)
        total_rss = sum(memories)
        p95_rss = statistics.quantiles(memories, n=20)[18] if len(memories) >= 20 else max(memories)
        sent = sum(kv_number(node, "sync_bytes_sent") for node in nodes)
        received = sum(kv_number(node, "sync_bytes_received") for node in nodes)
        peer_counts = []
        for node in nodes:
            with sqlite3.connect(node.db, timeout=5) as conn:
                peer_counts.append(conn.execute("SELECT COUNT(*) FROM peers").fetchone()[0])
        check("ни один узел не превысил peer limit", max(peer_counts) <= 32, f"max={max(peer_counts)}")
        print("\n===== SCALE METRICS =====")
        print(f"nodes={count}")
        print(f"startup_and_bootstrap_seconds={time.monotonic() - started:.1f}")
        print(f"initial_convergence_seconds={convergence_seconds:.1f}")
        print(f"leaf_to_root_seconds={propagation_seconds:.1f}")
        print(f"rss_total_mib={total_rss / 1024:.1f}")
        print(f"rss_p95_mib={p95_rss / 1024:.1f}")
        print(f"sync_sent_mib={sent / 1024 / 1024:.2f}")
        print(f"sync_received_mib={received / 1024 / 1024:.2f}")
        print(f"direct_peers_max={max(peer_counts)}")
    finally:
        for node in nodes:
            node.stop()

    print("\n===== SCALE ИТОГ =====")
    print("failed:", len(failures))
    for failure in failures:
        print(" -", failure)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
