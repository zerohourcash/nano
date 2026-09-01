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
import base64
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
from device_test_signing import CRITICAL, DeviceSigner

ROOT = Path(__file__).resolve().parent.parent
EXE = "meshkeeper-node.exe" if os.name == "nt" else "meshkeeper-node"
BINARY = ROOT / "backend" / "target" / "release" / EXE
FALLBACK = ROOT / "dist" / "server" / EXE

TOKEN = "test-sync-token-of-at-least-32-characters"
OWNER_PHONE = "+7 900 111-22-33"
OWNER_PASSWORD = "SuperSecret123"
PHOTO_BYTES = bytes((index * 31) % 256 for index in range(150_000))
PHOTO_DATA_URL = "data:image/png;base64," + base64.b64encode(PHOTO_BYTES).decode()
DOCUMENT_BYTES = b"%PDF-1.7\n" + bytes((index * 17) % 256 for index in range(90_000))
DOCUMENT_DATA_URL = "data:application/pdf;base64," + base64.b64encode(DOCUMENT_BYTES).decode()
KNOWLEDGE_DATA_URL = "data:text/plain;base64,0J/RgNC+0LLQtdGA0LrQsA=="
CHAT_DATA_URL = "data:text/plain;base64," + base64.b64encode(b"offline chat file").decode()

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
        self.signer = DeviceSigner(name)
        self.device_registered = False
        self.env = {
            **os.environ,
            "MESHKEEPER_DB": str(self.db),
            "MESHKEEPER_BIND": f"127.0.0.1:{port}",
            "MESHKEEPER_DEMO_DATA": "0",
            "MESHKEEPER_DEMO_LOGIN": "0",
            "MESHKEEPER_STRICT_NODE_TRUST": "0",
            **env_extra,
        }
        self.proc = self._spawn()

    def _spawn(self):
        return subprocess.Popen(
            [str(binary())], cwd=str(ROOT), env=self.env,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )

    def restart(self) -> None:
        if self.proc.poll() is None:
            raise RuntimeError(f"{self.name} ещё работает")
        self.proc = self._spawn()

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
            if proc in CRITICAL:
                if not self.device_registered:
                    enrolled = self.call("auth.registerDevice", {
                        "deviceId": self.signer.device_id,
                        "name": self.name,
                        "publicKey": self.signer.public_key,
                    })
                    if not isinstance(enrolled, dict) or enrolled.get("deviceId") != self.signer.device_id:
                        return {"__err": "device enrollment failed", "__detail": enrolled}
                    self.device_registered = True
                for key, value in self.signer.headers(f"/api/trpc/{proc}", body.encode()).items():
                    req.add_header(key, value)
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

    def checkout_payload(self, item_id: int, **fields):
        issued = self.call("items.qrLabel", {"itemId": item_id}, mutation=False)
        if not isinstance(issued, dict) or not issued.get("label"):
            raise AssertionError(f"signed QR issuance failed for item {item_id}: {issued}")
        return {"itemId": item_id, "qrLabel": issued["label"], **fields}

    def stop(self, cleanup: bool = True) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        if cleanup:
            for suffix in ("", "-wal", "-shm"):
                Path(str(self.db) + suffix).unlink(missing_ok=True)


def binary() -> Path:
    candidates = [path for path in (BINARY, FALLBACK) if path.is_file()]
    return max(candidates, key=lambda path: path.stat().st_mtime) if candidates else BINARY


def titles(node: Node, workspace_id: int) -> list[str]:
    rows = node.call("reports.allItems", {"workspaceId": workspace_id}, mutation=False)
    return [r["title"] for r in rows] if isinstance(rows, list) else []


def item_named(node: Node, workspace_id: int, title: str):
    rows = node.call("reports.allItems", {"workspaceId": workspace_id}, mutation=False)
    if not isinstance(rows, list):
        return None
    return next((row for row in rows if row.get("title") == title), None)


def item_full_named(node: Node, workspace_id: int, title: str):
    summary = item_named(node, workspace_id, title)
    if not isinstance(summary, dict) or not summary.get("id"):
        return None
    detail = node.call("items.byId", {"id": summary["id"]}, mutation=False)
    return detail if isinstance(detail, dict) else None


def pull_delta(node: Node, frontier: list[dict]) -> dict:
    request = urllib.request.Request(
        f"{node.base}/sync/journal/pull",
        data=json.dumps({"frontier": frontier}).encode(),
        method="POST",
    )
    request.add_header("authorization", f"Bearer {TOKEN}")
    request.add_header("content-type", "application/json")
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.loads(response.read().decode())


def session_call(base: str, opener, proc: str, payload=None, mutation: bool = True):
    """Небольшой независимый клиент для проверки ACL второй сессией."""
    body = json.dumps({"0": {"json": payload}})
    if mutation:
        request = urllib.request.Request(
            f"{base}/api/trpc/{proc}?batch=1",
            data=body.encode(),
            method="POST",
        )
        request.add_header("content-type", "application/json")
    else:
        query = urllib.parse.quote(body)
        request = urllib.request.Request(
            f"{base}/api/trpc/{proc}?batch=1&input={query}", method="GET"
        )
    request.add_header("origin", base)
    with opener.open(request, timeout=20) as response:
        data = json.loads(response.read().decode())
    if isinstance(data, list):
        data = data[0]
    if "error" in data:
        return {"__err": data["error"]["json"].get("message")}
    return data["result"]["data"]["json"]


def journal_from(node: Node) -> dict:
    request = urllib.request.Request(f"{node.base}/sync/journal")
    request.add_header("authorization", f"Bearer {TOKEN}")
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.loads(response.read().decode())


def push_journal(node: Node, journal: dict) -> dict:
    request = urllib.request.Request(
        f"{node.base}/sync/journal",
        data=json.dumps(journal).encode(),
        method="POST",
    )
    request.add_header("authorization", f"Bearer {TOKEN}")
    request.add_header("content-type", "application/json")
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.loads(response.read().decode())


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
    # Архивный узел добровольно хранит все обнаруженные CAS-объекты. Обычные
    # телефоны остаются metadata/smart-узлами и не обязаны копировать файлы.
    server = Node(
        "server",
        server_port,
        {"MESHKEEPER_SYNC_TOKEN": TOKEN, "MESHKEEPER_CONTENT_MODE": "full"},
    )
    node = Node(
        "node",
        free_port(),
        {
            "MESHKEEPER_SYNC_TOKEN": TOKEN,
            "MESHKEEPER_UPSTREAM": f"http://127.0.0.1:{server_port}",
            "MESHKEEPER_SYNC_INTERVAL": "5",
            "MESHKEEPER_CONTENT_MODE": "metadata",
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
        check("узлы используют подписанные account-chain", srv_health.get("journal") == "signed-account-chains" and node_health.get("journal") == "signed-account-chains", str(srv_health))

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
        server_item = server.call(
            "items.create",
            {
                "workspaceId": ws_id,
                "title": "Перфоратор с сервера",
                "storageId": st_id,
                "sourceSystem": "facekit",
                "externalId": "2877042",
                "metadata": {"ownerCompany": "ООО ФейсКИТ", "labels": ["mesh"]},
                "photos": [{"url": PHOTO_DATA_URL, "thumbUrl": PHOTO_DATA_URL}],
            },
        )
        division = server.call(
            "admin.organizationNodes.create",
            {
                "workspaceId": ws_id,
                "kind": "division",
                "name": "Производство",
                "tabLabel": "Цеха",
            },
        )
        room = server.call(
            "admin.organizationNodes.create",
            {
                "workspaceId": ws_id,
                "parentId": division["id"],
                "kind": "room",
                "name": "Кабинет 204",
            },
        )
        server.call(
            "items.update",
            {"id": server_item["id"], "organizationNodeId": room["id"]},
        )

        mesh_invite = server.call(
            "admin.workspaces.createInvite",
            {"workspaceId": ws_id, "role": "viewer", "maxUses": 1},
        )
        journal_req = urllib.request.Request(f"{server.base}/sync/journal")
        journal_req.add_header("authorization", f"Bearer {TOKEN}")
        with urllib.request.urlopen(journal_req, timeout=5) as response:
            journal = json.loads(response.read().decode())
        synced_invites = journal.get("invites", [])
        check(
            "приглашение экспортируется только как SHA-256 capability",
            len(synced_invites) == 1
            and len(synced_invites[0].get("tokenDigest", "")) == 64
            and mesh_invite["token"] not in json.dumps(journal),
            str(synced_invites),
        )
        administrative_events = [
            event
            for event in journal.get("history", [])
            if event.get("type")
            in {"organization_node_create", "invitation_create"}
        ]
        check(
            "структура и приглашение привязаны к Ed25519 device-proof администратора",
            len(administrative_events) == 3
            and all(event.get("requestDeviceId") for event in administrative_events)
            and all(event.get("requestSignature") for event in administrative_events),
            str(administrative_events)[:300],
        )
        item_events = [
            event
            for event in journal.get("history", [])
            if event.get("type") in {"item_state_create", "item_state_update"}
            and event.get("itemGuid") == server_item.get("guid")
        ]
        check(
            "создание и изменение карточки имеют переносимый device-proof",
            len(item_events) == 2
            and all(event.get("requestDeviceId") for event in item_events)
            and all(event.get("requestSignature") for event in item_events),
            str(item_events)[:300],
        )

        initial_pull = push_journal(node, journal)
        check("первичный pull принят криптографическим импортом", initial_pull.get("ok") is True, str(initial_pull))

        print("\n== 4. Вход на узле офлайн ==")
        # Каталог закрыт без сессии, поэтому сначала логин — как только узел
        # увидел сотрудника, пришедшего с сервера.
        logged_in = wait_for(
            lambda: "id"
            in node.call("auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD})
        )
        check("синхронизированный сотрудник входит на узле", logged_in)
        invite_available_offline = wait_for(
            lambda: node.call("auth.inviteInfo", {"token": mesh_invite["token"]}).get("role")
            == "viewer"
        )
        check("QR-приглашение проверяется на узле после pull", invite_available_offline)

        node_ws = node.call("meta.workspaces", None, mutation=False)
        node_ws_id = node_ws[0]["id"] if isinstance(node_ws, list) and node_ws else None
        check("пространство доехало до узла", bool(node_ws_id), str(node_ws)[:180])
        structure = node.call(
            "admin.organizationNodes.list",
            {"workspaceId": node_ws_id},
            mutation=False,
        )
        check(
            "настраиваемая структура и родительская связь синхронизировались",
            isinstance(structure, list)
            and len(structure) == 2
            and any(row.get("parentId") for row in structure),
            str(structure)[:240],
        )

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
        check(
            "оборудование привязано к разделу после синхронизации",
            isinstance(synced_card, dict)
            and synced_card.get("organizationNode", {}).get("name") == "Кабинет 204",
            str(synced_card.get("organizationNode") if isinstance(synced_card, dict) else synced_card),
        )
        metadata_status = node.call("content.status", None, mutation=False)
        check(
            "metadata-нода получила летопись и CAS-каталог без тяжёлого файла",
            metadata_status.get("mode") == "metadata"
            and metadata_status.get("missing") == 1
            and metadata_status.get("blobs") == 0,
            str(metadata_status),
        )
        full_status = node.call("content.setMode", {"mode": "full"})
        check(
            "любой авторизованный узел переключается в полную ноду подписанной командой",
            full_status.get("mode") == "full",
            str(full_status),
        )
        node.call("sync.pullNow", {})
        photo_arrived = wait_for(
            lambda: (item_named(node, node_ws_id or 1, "Перфоратор с сервера") or {})
            .get("photos", [{}])[0]
            .get("url")
            == PHOTO_DATA_URL,
            timeout=30,
        )
        check("многочастное CAS-фото докачалось и прошло SHA-256", photo_arrived)
        replicated_status = node.call("content.status", None, mutation=False)
        check(
            "полная нода восстановила все известные CAS-файлы",
            replicated_status.get("missing") == 0
            and replicated_status.get("blobs") == replicated_status.get("catalogEntries") == 1,
            str(replicated_status),
        )
        compact_snapshot = journal_from(node)
        check(
            "snapshot содержит только CAS-ссылку и manifest, не base64 файла",
            len(compact_snapshot.get("blobs", [])) == 1
            and compact_snapshot.get("photos", [{}])[0].get("url", "").startswith("cas:")
            and PHOTO_DATA_URL not in json.dumps(compact_snapshot),
        )

        tombstone_item = server.call(
            "items.create",
            {"workspaceId": ws_id, "title": "Не воскресать после офлайна"},
        )
        server.call("sync.pullNow", {})
        check(
            "карточка дошла до ноды до разделения сети",
            wait_for(lambda: "Не воскресать после офлайна" in titles(node, node_ws_id or 1)),
        )
        stale_active_snapshot = journal_from(node)
        archived = server.call("items.remove", {"id": tombstone_item["id"]})
        check(
            "удаление стало подписанным monotonic tombstone",
            archived.get("tombstone", {}).get("tombstoneHash") is not None,
            str(archived)[:220],
        )
        stale_result = push_journal(server, stale_active_snapshot)
        check(
            "старый активный snapshot принят идемпотентно, но не воскресил ТМЦ",
            stale_result.get("ok") is True
            and "Не воскресать после офлайна" not in titles(server, ws_id),
            str(stale_result),
        )
        server.call("sync.pullNow", {})
        check(
            "tombstone скрыл карточку и на вернувшейся офлайн-ноде",
            wait_for(lambda: "Не воскресать после офлайна" not in titles(node, node_ws_id or 1)),
        )
        tombstone_snapshot = journal_from(node)
        check(
            "tombstone и исходная история доступны на восстановленной ноде",
            len(tombstone_snapshot.get("itemTombstones", [])) == 1
            and any(event.get("type") == "item_archive" for event in tombstone_snapshot.get("history", [])),
            str(tombstone_snapshot.get("itemTombstones")),
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

        # Реальный offline-разрыв: upstream-процесс недоступен, но телефонная
        # нода продолжает принимать подписанные текстовые транзакции и CAS bytes.
        server.stop(cleanup=False)
        offline_blob = node.call(
            "content.ingest",
            {"workspaceId": node_ws_id, "dataUrl": DOCUMENT_DATA_URL},
        )
        document_guid = str(uuid.uuid4())
        offline_document = node.call(
            "items.addDocument",
            {
                "itemId": created["id"],
                "itemGuid": created["guid"],
                "documentGuid": document_guid,
                "name": "Офлайн-паспорт.pdf",
                "url": offline_blob.get("url"),
                "mime": "application/pdf",
                "accessLevel": "accounting",
            },
        )
        offline_snapshot = journal_from(node)
        portable_document = next(
            (row for row in offline_snapshot.get("documents", []) if row.get("guid") == document_guid),
            None,
        )
        document_event = next(
            (event for event in offline_snapshot.get("history", []) if event.get("type") == "document_add" and event.get("fromLabel") == document_guid),
            None,
        )
        check(
            "при недоступном upstream документ остаётся локальной signed-транзакцией",
            offline_document.get("guid") == document_guid
            and isinstance(portable_document, dict)
            and portable_document.get("url", "").startswith("cas:")
            and portable_document.get("accessLevel") == "accounting"
            and isinstance(document_event, dict)
            and document_event.get("eventVersion") == 3
            and bool(document_event.get("requestDeviceId")),
            str({"document": portable_document, "event": document_event})[:600],
        )
        check(
            "offline snapshot не содержит байты документа",
            DOCUMENT_DATA_URL not in json.dumps(offline_snapshot)
            and base64.b64encode(DOCUMENT_BYTES[:256]).decode() not in json.dumps(offline_snapshot),
        )

        server.restart()
        check("upstream восстановился после разрыва", server.wait_ready())
        node.call("sync.pullNow", {})
        remote_document_arrived = wait_for(
            lambda: any(
                document.get("guid") == document_guid
                and document.get("url") == DOCUMENT_DATA_URL
                and document.get("accessLevel") == "accounting"
                for document in (item_full_named(server, ws_id, "Шуруповёрт с узла") or {}).get("documents", [])
            ),
            timeout=30,
        )
        check(
            "после восстановления full-node получил транзакцию и CAS-документ",
            remote_document_arrived,
            str({
                "documents": (item_full_named(server, ws_id, "Шуруповёрт с узла") or {}).get("documents", []),
                "content": server.call("content.status", None, mutation=False),
            })[:500],
        )
        server_snapshot = journal_from(server)
        restored_document = next(
            (row for row in server_snapshot.get("documents", []) if row.get("guid") == document_guid),
            None,
        )
        check(
            "ACL и ledgerHash документа пережили offline-синхронизацию",
            isinstance(restored_document, dict)
            and restored_document.get("accessLevel") == "accounting"
            and restored_document.get("url", "").startswith("cas:")
            and bool(restored_document.get("ledgerHash")),
            str(restored_document),
        )
        viewer_opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(CookieJar())
        )
        viewer_joined = session_call(
            server.base,
            viewer_opener,
            "auth.joinRegister",
            {
                "token": mesh_invite["token"],
                "fullName": "Удалённый наблюдатель",
                "phone": "+7 900 777-00-01",
                "password": "ViewerSecret123",
            },
        )
        remote_summary = item_named(server, ws_id, "Шуруповёрт с узла") or {}
        viewer_card = session_call(
            server.base,
            viewer_opener,
            "items.byId",
            {"id": remote_summary.get("id")},
            mutation=False,
        )
        check(
            "удалённая viewer-сессия не получает защищённые документы",
            bool(viewer_joined.get("id"))
            and isinstance(viewer_card, dict)
            and "documents" not in viewer_card,
            str(viewer_card)[:300],
        )
        stale_document_replay = push_journal(server, offline_snapshot)
        check(
            "запоздавший повтор offline-snapshot не откатывает full-node",
            stale_document_replay.get("ok") is False
            and any(
                marker in str(stale_document_replay.get("error", "")).lower()
                for marker in ("rollback", "sequence", "откат")
            )
            and bool((item_full_named(server, ws_id, "Шуруповёрт с узла") or {}).get("documents")),
            str(stale_document_replay),
        )
        material = node.call(
            "items.create",
            {
                "workspaceId": node_ws_id,
                "title": "Кабель бухта с узла",
                "quantitative": True,
                "quantity": 10,
                "unit": "м",
            },
        )
        held_material = node.call(
            "transfers.take",
            node.checkout_payload(material["id"], quantity=4, dueAt="2026-09-10T12:00:00Z"),
        )
        check(
            "количественная QR-выдача создала локальную custody-проводку",
            held_material.get("issuedQty") == 4 and held_material.get("stockQty") == 6,
            str(held_material)[:180],
        )
        inventory = node.call("inventory.create", {"workspaceId": node_ws_id})
        node.call("inventory.checkItem", {
            "sessionId": inventory["id"], "itemId": material["id"],
            "checked": True, "actualQty": 6,
        })
        inventory_done = node.call("inventory.complete", {"sessionId": inventory["id"]})
        inventory_records = journal_from(node).get("inventoryRecords", [])
        check(
            "офлайн-инвентаризация записана переносимыми подписанными фактами",
            bool(inventory_done.get("completedAt"))
            and len(inventory_records) == 3
            and {row.get("kind") for row in inventory_records} == {"create", "check", "complete"},
            str({"completedAt": inventory_done.get("completedAt"), "recordHash": inventory_done.get("recordHash"), "records": inventory_records})[:1000],
        )
        node_me = node.call("meta.currentUser", None, mutation=False)
        minted = node.call(
            "bit.mint",
            {"workspaceId": node_ws_id, "recipientUserId": node_me["id"], "amount": 100, "memo": "Офлайн-эмиссия"},
        )
        check("офлайн-эмиссия Bit записана двойной проводкой", minted.get("status") == "posted", str(minted))
        knowledge = node.call(
            "knowledge.save",
            {
                "workspaceId": node_ws_id,
                "slug": "offline/safety",
                "title": "Офлайн-инструкция",
                "content": "# Безопасность\nПроверить инструмент перед работой.",
                "attachments": [{"name": "Памятка", "url": KNOWLEDGE_DATA_URL}],
            },
        )
        check("ревизия локальной базы знаний подписана", bool(knowledge.get("savedRevisionHash")), str(knowledge)[:180])
        knowledge_snapshot = journal_from(node)
        exported_attachment = (
            knowledge_snapshot.get("knowledge", {})
            .get("revisions", [{}])[-1]
            .get("attachments", [{}])[0]
        )
        check(
            "wiki snapshot передаёт CAS-ссылку без байтов вложения",
            exported_attachment.get("url", "").startswith("cas:")
            and KNOWLEDGE_DATA_URL not in json.dumps(knowledge_snapshot),
            str(exported_attachment),
        )
        chat_blob = node.call(
            "content.ingest",
            {"workspaceId": node_ws_id, "dataUrl": CHAT_DATA_URL},
        )
        chat_guid = str(uuid.uuid4())
        offline_chat = node.call(
            "chat.send",
            {
                "workspaceId": node_ws_id,
                "workspaceGuid": node_ws[0]["guid"],
                "messageGuid": chat_guid,
                "text": "Офлайн-файл для смены",
                "attachments": [{
                    "name": "shift.txt",
                    "url": chat_blob["url"],
                    "mime": "text/plain",
                }],
            },
        )
        chat_snapshot = journal_from(node)
        exported_chat = next(
            (row for row in chat_snapshot.get("messages", []) if row.get("guid") == chat_guid),
            None,
        )
        check(
            "чат хранит файл как compact V3 CAS intent",
            offline_chat.get("guid") == chat_guid
            and isinstance(exported_chat, dict)
            and exported_chat.get("attachments", [{}])[0].get("url", "").startswith("cas:")
            and CHAT_DATA_URL not in json.dumps(chat_snapshot),
            str(exported_chat),
        )
        fault = node.call(
            "items.reportFault",
            {"itemId": created["id"], "severity": "high", "description": "Офлайн: искрит выключатель"},
        )
        resolved_fault = node.call(
            "items.resolveFault",
            {"id": fault["id"], "status": "resolved", "comment": "Выключатель заменён локально"},
        )
        check(
            "офлайн lifecycle неисправности связан с двумя Ledger-транзакциями",
            bool(fault.get("recordHash")) and bool(resolved_fault.get("recordHash")),
            str(resolved_fault)[:180],
        )
        change = node.call(
            "items.requestChange",
            {"itemId": created["id"], "payload": {"comment": "Проверено офлайн"}, "comment": "Добавить отметку"},
        )
        change_decision = node.call(
            "items.decideChange",
            {"id": change["id"], "accept": True, "reason": "Подтверждено локально"},
        )
        check(
            "офлайн заявка и решение связаны с Ledger",
            bool(change.get("recordHash")) and bool(change_decision.get("recordHash")),
            str(change_decision)[:180],
        )
        node.call("sync.pullNow", {})
        back = wait_for(lambda: "Шуруповёрт с узла" in titles(server, ws_id))
        check("предмет с узла доехал до сервера", back, str(titles(server, ws_id))[:160])
        restored_inventory = None
        def inventory_arrived():
            nonlocal restored_inventory
            rows = server.call("inventory.sessions", {"workspaceId": ws_id}, mutation=False)
            found = next((row for row in rows if row.get("number") == inventory.get("number")), None) if isinstance(rows, list) else None
            restored_inventory = server.call("inventory.byId", {"id": found["id"]}, mutation=False) if found else None
            return isinstance(restored_inventory, dict) and restored_inventory.get("status") == "completed"
        inventory_synced = wait_for(inventory_arrived, timeout=30)
        check(
            "полная нода восстановила сессию и результаты офлайн-инвентаризации",
            inventory_synced
            and any(row.get("item", {}).get("title") == "Кабель бухта с узла" and row.get("checked") for row in restored_inventory.get("results", [])),
            str(restored_inventory)[:240],
        )
        back_card = item_named(server, ws_id, "Шуруповёрт с узла")
        check(
            "metadata с узла доехала до сервера",
            isinstance(back_card, dict)
            and back_card.get("externalId") == "node-1"
            and back_card.get("metadata", {}).get("isKit") is True,
            str(back_card)[:180],
        )
        custody_synced = wait_for(
            lambda: (
                (item_named(server, ws_id, "Кабель бухта с узла") or {}).get("issuedQty") == 4
                and (item_named(server, ws_id, "Кабель бухта с узла") or {}).get("stockQty") == 6
            )
        )
        restored_material = item_named(server, ws_id, "Кабель бухта с узла") or {}
        check(
            "полная нода восстановила кто держит 4 единицы из custody-летописи",
            custody_synced
            and len(restored_material.get("holders", [])) == 1
            and restored_material.get("holders", [{}])[0].get("quantity") == 4,
            str({key: restored_material.get(key) for key in ("issuedQty", "stockQty", "quantity", "holders")}),
        )
        bit_synced = wait_for(
            lambda: server.call("bit.balance", {"workspaceId": ws_id, "userId": owner["id"]}, mutation=False).get("balance") == 100
        )
        check("подписанная бухгалтерская проводка Bit дошла до сервера", bit_synced)
        synced_knowledge: dict = {}

        def knowledge_arrived() -> bool:
            nonlocal synced_knowledge
            synced_knowledge = server.call(
                "knowledge.bySlug",
                {"workspaceId": ws_id, "slug": "offline/safety"},
                mutation=False,
            )
            attachments = synced_knowledge.get("current", {}).get("attachments", [])
            return (
                synced_knowledge.get("current", {}).get("content", "").startswith("# Безопасность")
                and bool(attachments)
                and attachments[0].get("url", "").startswith("data:text/plain")
            )

        check(
            "текст и CAS-вложение базы знаний дошли до сервера",
            wait_for(knowledge_arrived, timeout=30),
            str(synced_knowledge)[:220],
        )
        synced_chat = wait_for(
            lambda: any(
                message.get("guid") == chat_guid
                and message.get("attachments", [{}])[0].get("url") == CHAT_DATA_URL
                for message in server.call("chat.list", {"workspaceId": ws_id}, mutation=False)
            ),
            timeout=30,
        )
        check("full-node восстановил CAS-вложение offline-чата", synced_chat)
        synced_faults = server.call("items.faults", {"workspaceId": ws_id}, mutation=False)
        check(
            "полная нода восстановила описание и решение офлайн-неисправности",
            isinstance(synced_faults, list)
            and any(
                row.get("guid") == fault.get("guid")
                and row.get("description") == "Офлайн: искрит выключатель"
                and row.get("resolution") == "Выключатель заменён локально"
                and row.get("status") == "resolved"
                for row in synced_faults
            ),
            str(synced_faults)[:260],
        )
        synced_changes = server.call("items.changeRequests", {"workspaceId": ws_id}, mutation=False)
        changed_card = item_named(server, ws_id, "Шуруповёрт с узла") or {}
        check(
            "полная нода восстановила офлайн-заявку, решение и применённый patch",
            isinstance(synced_changes, list)
            and any(row.get("guid") == change.get("guid") and row.get("status") == "accepted" for row in synced_changes)
            and changed_card.get("comment") == "Проверено офлайн",
            str(synced_changes)[:260],
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

        print("\n== 8. Frontier / инкрементальный обмен ==")
        node_snapshot = journal_from(node)
        empty_delta = pull_delta(server, node_snapshot.get("frontier", []))
        check(
            "повторный обмен не пересылает известную историю",
            empty_delta.get("historyMode") == "delta"
            and empty_delta.get("history") == [],
            str(empty_delta.get("history"))[:200],
        )
        server.call(
            "items.update",
            {"id": server_item["id"], "comment": "Новое изменение после frontier"},
        )
        one_delta = pull_delta(server, node_snapshot.get("frontier", []))
        check(
            "после frontier передаётся только новое событие",
            one_delta.get("historyMode") == "delta"
            and len(one_delta.get("history", [])) == 1
            and one_delta["history"][0].get("prevHash")
            in {entry.get("head") for entry in node_snapshot.get("frontier", [])},
            str(one_delta.get("history"))[:260],
        )
        for index in range(80):
            updated = server.call(
                "items.update",
                {"id": server_item["id"], "comment": f"Длинная история #{index:03d}"},
            )
            if not isinstance(updated, dict) or "id" not in updated:
                break
        long_snapshot = journal_from(server)
        steady_delta = pull_delta(server, long_snapshot.get("frontier", []))
        full_bytes = len(json.dumps(long_snapshot, separators=(",", ":")).encode())
        delta_bytes = len(json.dumps(steady_delta, separators=(",", ":")).encode())
        check(
            "steady-state delta существенно меньше длинной полной истории",
            steady_delta.get("history") == [] and delta_bytes < full_bytes * 0.35,
            f"full={full_bytes} delta={delta_bytes} ratio={delta_bytes / full_bytes:.3f}",
        )
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
