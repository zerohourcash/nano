"""Сквозная проверка связки «центральный сервер + локальный узел».

Поднимает два экземпляра узла на временных базах: один в роли сервера
(есть общий токен, нет upstream), второй в роли локального узла
(тот же токен + MESHKEEPER_UPSTREAM на сервер). Проверяет, что данные
расходятся в обе стороны, что сотрудник может войти на узле офлайн и что
обмен закрыт без токена.

    python scripts/sync_test.py

Требуется собранный узел: npm run build
"""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from http.cookiejar import CookieJar
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = "meshkeeper-node.exe" if os.name == "nt" else "meshkeeper-node"
BINARY = ROOT / "backend" / "target" / "release" / EXE
FALLBACK = ROOT / "dist" / "server" / EXE

TOKEN = "test-sync-token-of-at-least-32-characters"
OWNER_PHONE = "+7 900 111-22-33"
OWNER_PASSWORD = "SuperSecret123"

# Консоль Windows по умолчанию не в UTF-8: без этого падает первый же вывод.
for stream in (sys.stdout, sys.stderr):
    reconfigure = getattr(stream, "reconfigure", None)
    if reconfigure is not None:
        reconfigure(encoding="utf-8", errors="replace")

failures: list[str] = []


def check(label: str, ok: bool, detail: str = "") -> None:
    print(f"[{'OK  ' if ok else 'FAIL'}] {label} {detail}")
    if not ok:
        failures.append(label)


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class Node:
    """Экземпляр узла с собственной базой и куками."""

    def __init__(self, name: str, port: int, env_extra: dict[str, str]):
        self.name = name
        self.port = port
        self.base = f"http://127.0.0.1:{port}"
        self.db = Path(tempfile.gettempdir()) / f"meshkeeper-{name}-{uuid.uuid4().hex}.db"
        self.cj = CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.cj)
        )
        env = {
            **os.environ,
            "MESHKEEPER_DB": str(self.db),
            "MESHKEEPER_BIND": f"127.0.0.1:{port}",
            "MESHKEEPER_DEMO_DATA": "0",
            "MESHKEEPER_DEMO_LOGIN": "0",
            **env_extra,
        }
        self.proc = subprocess.Popen(
            [str(binary())],
            cwd=str(ROOT),
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    def wait_ready(self, timeout: float = 25.0) -> bool:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                with urllib.request.urlopen(f"{self.base}/health", timeout=2):
                    return True
            except (urllib.error.URLError, OSError):
                time.sleep(0.25)
        return False

    def call(self, proc: str, payload=None, mutation: bool = True):
        body = json.dumps({"0": {"json": payload}})
        if mutation:
            req = urllib.request.Request(
                f"{self.base}/api/trpc/{proc}?batch=1",
                data=body.encode(),
                method="POST",
            )
            req.add_header("content-type", "application/json")
        else:
            query = urllib.parse.quote(body)
            req = urllib.request.Request(
                f"{self.base}/api/trpc/{proc}?batch=1&input={query}", method="GET"
            )
        req.add_header("origin", self.base)
        try:
            with self.opener.open(req, timeout=20) as resp:
                data = json.loads(resp.read().decode())
        except urllib.error.HTTPError as exc:
            return {"__http": exc.code}
        if isinstance(data, list):
            data = data[0]
        if "error" in data:
            return {"__err": data["error"]["json"].get("message")}
        return data["result"]["data"]["json"]

    def stop(self) -> None:
        self.proc.terminate()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        for suffix in ("", "-wal", "-shm"):
            Path(str(self.db) + suffix).unlink(missing_ok=True)


def binary() -> Path:
    return BINARY if BINARY.is_file() else FALLBACK


def titles(node: Node, workspace_id: int) -> list[str]:
    rows = node.call("reports.allItems", {"workspaceId": workspace_id}, mutation=False)
    return [r["title"] for r in rows] if isinstance(rows, list) else []


def item_named(node: Node, workspace_id: int, title: str):
    rows = node.call("reports.allItems", {"workspaceId": workspace_id}, mutation=False)
    if not isinstance(rows, list):
        return None
    return next((row for row in rows if row.get("title") == title), None)


def wait_for(predicate, timeout: float = 40.0, step: float = 1.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if predicate():
            return True
        time.sleep(step)
    return False


def main() -> int:
    if not binary().is_file():
        print(f"Узел не собран: нет {binary()}. Выполните npm run build", file=sys.stderr)
        return 2

    server_port = free_port()
    server = Node("server", server_port, {"MESHKEEPER_SYNC_TOKEN": TOKEN})
    node = Node(
        "node",
        free_port(),
        {
            "MESHKEEPER_SYNC_TOKEN": TOKEN,
            "MESHKEEPER_UPSTREAM": f"http://127.0.0.1:{server_port}",
            "MESHKEEPER_SYNC_INTERVAL": "5",
        },
    )
    try:
        if not server.wait_ready() or not node.wait_ready():
            print("Узлы не поднялись", file=sys.stderr)
            return 1

        print("== 1. Роли ==")
        srv_health = json.loads(
            urllib.request.urlopen(f"{server.base}/health", timeout=5).read().decode()
        )
        node_health = json.loads(
            urllib.request.urlopen(f"{node.base}/health", timeout=5).read().decode()
        )
        check("сервер объявляет роль server", srv_health.get("role") == "server", str(srv_health))
        check("узел объявляет роль node", node_health.get("role") == "node", str(node_health))

        print("\n== 2. Обмен закрыт без токена ==")
        try:
            urllib.request.urlopen(f"{server.base}/sync/journal", timeout=5)
            check("обмен без токена отклонён", False, "ответ 200")
        except urllib.error.HTTPError as exc:
            check("обмен без токена отклонён", exc.code == 401, f"status={exc.code}")

        req = urllib.request.Request(f"{server.base}/sync/journal")
        req.add_header("authorization", "Bearer wrong-token-wrong-token-wrong-token")
        try:
            urllib.request.urlopen(req, timeout=5)
            check("обмен с чужим токеном отклонён", False, "ответ 200")
        except urllib.error.HTTPError as exc:
            check("обмен с чужим токеном отклонён", exc.code == 401, f"status={exc.code}")

        print("\n== 3. Сервер → узел ==")
        owner = server.call(
            "auth.register",
            {
                "fullName": "Дима Владелец",
                "phone": OWNER_PHONE,
                "password": OWNER_PASSWORD,
                "workspaceName": "Объект Северный",
            },
        )
        check("владелец создан на сервере", isinstance(owner, dict) and "id" in owner, str(owner)[:120])
        ws = server.call("meta.workspaces", None, mutation=False)
        ws_id = ws[0]["id"] if isinstance(ws, list) and ws else None
        storages = server.call("admin.storages.list", {"workspaceId": ws_id}, mutation=False)
        st_id = storages[0]["id"] if isinstance(storages, list) and storages else None
        server.call(
            "items.create",
            {
                "workspaceId": ws_id,
                "title": "Перфоратор с сервера",
                "storageId": st_id,
                "sourceSystem": "facekit",
                "externalId": "2877042",
                "metadata": {"ownerCompany": "ООО ФейсКИТ", "labels": ["mesh"]},
            },
        )

        server.call(
            "admin.workspaces.createInvite",
            {"workspaceId": ws_id, "role": "viewer", "maxUses": 1},
        )
        journal_req = urllib.request.Request(f"{server.base}/sync/journal")
        journal_req.add_header("authorization", f"Bearer {TOKEN}")
        with urllib.request.urlopen(journal_req, timeout=5) as response:
            journal = json.loads(response.read().decode())
        check("активные приглашения не экспортируются", journal.get("invites") == [], str(journal.get("invites")))

        node.call("sync.pullNow", {})

        print("\n== 4. Вход на узле офлайн ==")
        # Каталог закрыт без сессии, поэтому сначала логин — как только узел
        # увидел сотрудника, пришедшего с сервера.
        logged_in = wait_for(
            lambda: "id"
            in node.call("auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD})
        )
        check("синхронизированный сотрудник входит на узле", logged_in)

        node_ws = node.call("meta.workspaces", None, mutation=False)
        node_ws_id = node_ws[0]["id"] if isinstance(node_ws, list) and node_ws else None
        check("пространство доехало до узла", bool(node_ws_id), str(node_ws)[:180])

        arrived = wait_for(
            lambda: "Перфоратор с сервера" in titles(node, node_ws_id or 1)
        )
        check(
            "предмет с сервера доехал до узла",
            arrived,
            str(titles(node, node_ws_id or 1))[:160],
        )
        synced_card = item_named(node, node_ws_id or 1, "Перфоратор с сервера")
        check(
            "расширенные поля FaceKit синхронизировались",
            isinstance(synced_card, dict)
            and synced_card.get("externalId") == "2877042"
            and synced_card.get("metadata", {}).get("ownerCompany") == "ООО ФейсКИТ",
            str(synced_card)[:180],
        )

        print("\n== 5. Узел → сервер ==")
        created = node.call(
            "items.create",
            {
                "workspaceId": node_ws_id,
                "title": "Шуруповёрт с узла",
                "sourceSystem": "offline-node",
                "externalId": "node-1",
                "metadata": {"isKit": True, "currency": "RUB"},
            },
        )
        check("предмет создан на узле", isinstance(created, dict) and "id" in created, str(created)[:140])
        node.call("sync.pullNow", {})
        back = wait_for(lambda: "Шуруповёрт с узла" in titles(server, ws_id))
        check("предмет с узла доехал до сервера", back, str(titles(server, ws_id))[:160])
        back_card = item_named(server, ws_id, "Шуруповёрт с узла")
        check(
            "metadata с узла доехала до сервера",
            isinstance(back_card, dict)
            and back_card.get("externalId") == "node-1"
            and back_card.get("metadata", {}).get("isKit") is True,
            str(back_card)[:180],
        )

        print("\n== 6. Статус синхронизации ==")
        status = node.call("sync.status", None, mutation=False)
        check("узел знает свой upstream", bool(status.get("upstream")), str(status.get("upstream")))
        check("зафиксировано время обмена", bool(status.get("lastSync")), str(status.get("lastSync")))
        check("ошибок обмена нет", not status.get("lastError"), str(status.get("lastError")))

        print("\n== 7. Журнал операций синхронизируется ==")
        hist = server.call("history.all", {"workspaceId": ws_id}, mutation=False)
        comments = json.dumps(hist, ensure_ascii=False) if isinstance(hist, list) else ""
        check("операция узла видна в журнале сервера", "Шуруповёрт с узла" in comments, comments[:160])
    finally:
        node.stop()
        server.stop()

    print("\n===== ИТОГ =====")
    print("failed:", len(failures))
    for item in failures:
        print(" -", item)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
