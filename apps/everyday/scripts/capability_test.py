"""Real HTTP isolation test for two organization capabilities on one node."""

from __future__ import annotations

import base64
import json
import sqlite3
import urllib.error
import urllib.request
import uuid

from sync_test import Node, TOKEN, free_port, journal_from, wait_for

TOKEN_A = "capability-a-" + "a" * 40
TOKEN_B = "capability-b-" + "b" * 40


def request_json(node: Node, path: str, token: str, body: dict | None = None):
    request = urllib.request.Request(
        f"{node.base}{path}",
        data=None if body is None else json.dumps(body).encode(),
        method="GET" if body is None else "POST",
    )
    request.add_header("authorization", f"Bearer {token}")
    if body is not None:
        request.add_header("content-type", "application/json")
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return response.status, json.loads(response.read().decode())
    except urllib.error.HTTPError as error:
        raw = error.read().decode()
        return error.code, json.loads(raw) if raw.startswith("{") else {"error": raw}


def main() -> int:
    failures: list[str] = []

    def check(label: str, condition: bool, detail=""):
        print(f"[{'OK  ' if condition else 'FAIL'}] {label} {detail}")
        if not condition:
            failures.append(label)

    node = Node("capability-http", free_port(), {"MESHKEEPER_SYNC_TOKEN": TOKEN})
    peer_a = None
    peer_b = None
    try:
        check("исходная нода запущена", node.wait_ready())
        owner = node.call("auth.register", {
            "fullName": "Владелец capability",
            "phone": "+7 900 700-00-01",
            "password": "CapabilitySecret123",
            "workspaceName": "Организация A",
        })
        check("владелец создан", isinstance(owner, dict) and owner.get("id") is not None)
        first = node.call("meta.workspaces", None, mutation=False)[0]
        second = node.call("admin.workspaces.create", {
            "name": "Организация B", "internalIdPrefix": "B-", "timezone": "Europe/Moscow",
        })
        check("две организации имеют GUID", bool(first.get("guid") and second.get("guid")), f"{first} {second}")

        photos = []
        chat_files = []
        for workspace, title, byte in [(first, "Только A", b"organization-a"), (second, "Только B", b"organization-b")]:
            photo = "data:text/plain;base64," + base64.b64encode(byte).decode()
            item = node.call("items.create", {"workspaceId": workspace["id"], "title": title, "photos": [photo]})
            check(f"создан предмет {title}", isinstance(item, dict) and item.get("id") is not None, str(item)[:120])
            photos.append(item["photos"][0]["sha256"])
            message_guid = str(uuid.uuid4())
            chat_data = "data:text/plain;base64," + base64.b64encode(b"chat-" + byte).decode()
            uploaded = node.call("content.ingest", {
                "workspaceId": workspace["id"], "purpose": "chat-attachment",
                "messageGuid": message_guid, "dataUrl": chat_data,
            })
            sent_message = node.call("chat.send", {
                "workspaceId": workspace["id"], "workspaceGuid": workspace["guid"],
                "messageGuid": message_guid, "text": f"Файл смены {title}",
                "attachments": [{"name": "shift.txt", "url": uploaded["url"], "mime": uploaded["mime"]}],
            })
            check(f"чат-файл {title} связан с signed сообщением",
                  sent_message.get("guid") == message_guid and len(uploaded.get("hash", "")) == 64)
            chat_files.append(uploaded["hash"])

        full = journal_from(node)
        node.stop(cleanup=False)
        node.env.pop("MESHKEEPER_SYNC_TOKEN", None)
        node.env["MESHKEEPER_SYNC_CAPABILITIES"] = json.dumps([
            {"token": TOKEN_A, "workspaces": [first["guid"]], "peers": []},
            {"token": TOKEN_B, "workspaces": [second["guid"]], "peers": []},
        ])
        node.restart()
        check("нода перезапущена с двумя capability", node.wait_ready())

        status_a, journal_a = request_json(node, "/sync/journal", TOKEN_A)
        status_b, journal_b = request_json(node, "/sync/journal", TOKEN_B)
        check("capability A получает только A", status_a == 200
              and [row["guid"] for row in journal_a["workspaces"]] == [first["guid"]]
              and [row["title"] for row in journal_a["items"]] == ["Только A"], str(journal_a.get("items")))
        check("capability B получает только B", status_b == 200
              and [row["guid"] for row in journal_b["workspaces"]] == [second["guid"]]
              and [row["title"] for row in journal_b["items"]] == ["Только B"], str(journal_b.get("items")))
        wrong_status, _ = request_json(node, "/sync/journal", "wrong-" + "x" * 40)
        check("неизвестный bearer отклонён", wrong_status == 401, wrong_status)

        forged_status, forged_answer = request_json(node, "/sync/journal", TOKEN_A, full)
        check("валидно подписанный широкий журнал отклонён capability A", forged_status == 403, str(forged_answer))
        own_blob_status, _ = request_json(node, f"/sync/blob/{photos[0]}?offset=0", TOKEN_A)
        foreign_blob_status, _ = request_json(node, f"/sync/blob/{photos[1]}?offset=0", TOKEN_A)
        check("свой CAS доступен capability", own_blob_status == 200, own_blob_status)
        check("чужой CAS недоступен даже по известному hash", foreign_blob_status == 403, foreign_blob_status)
        own_chat_status, _ = request_json(node, f"/sync/blob/{chat_files[0]}?offset=0", TOKEN_A)
        foreign_chat_status, _ = request_json(node, f"/sync/blob/{chat_files[1]}?offset=0", TOKEN_A)
        check("свой CAS-файл чата доступен capability", own_chat_status == 200, own_chat_status)
        check("чужой CAS-файл чата закрыт даже при известном hash", foreign_chat_status == 403,
              foreign_chat_status)

        sync_status = node.call("sync.status", None, mutation=False)
        check("панель показывает две capability", sync_status.get("workspaceScopeMode") == "capabilities"
              and sync_status.get("capabilityCount") == 2, str(sync_status))
        bundle = node.call("sync.exportBundle", {"workspaceGuid": first["guid"]}, mutation=False)
        check("offline bundle выбирает capability активной организации", bundle.get("version") == 2
              and bundle.get("ciphertext"), str(bundle)[:120])

        peer_a = Node("capability-peer-a", free_port(), {
            "MESHKEEPER_SYNC_TOKEN": TOKEN_A,
            "MESHKEEPER_SYNC_WORKSPACES": first["guid"],
            "MESHKEEPER_UPSTREAM": node.base,
            "MESHKEEPER_SYNC_INTERVAL": "5",
            "MESHKEEPER_CONTENT_MODE": "full",
        })
        peer_b = Node("capability-peer-b", free_port(), {
            "MESHKEEPER_SYNC_TOKEN": TOKEN_B,
            "MESHKEEPER_SYNC_WORKSPACES": second["guid"],
            "MESHKEEPER_UPSTREAM": node.base,
            "MESHKEEPER_SYNC_INTERVAL": "5",
            "MESHKEEPER_CONTENT_MODE": "full",
        })
        check("два scoped peer запущены", peer_a.wait_ready() and peer_b.wait_ready())

        def local_titles(peer: Node):
            with sqlite3.connect(peer.db) as db:
                return [row[0] for row in db.execute("SELECT title FROM items ORDER BY title")]

        converged = wait_for(lambda: local_titles(peer_a) == ["Только A"] and local_titles(peer_b) == ["Только B"], timeout=35)
        check("два peer-loop независимо получили только свои организации", converged,
              f"A={local_titles(peer_a)} B={local_titles(peer_b)}")
        with sqlite3.connect(peer_a.db) as db_a, sqlite3.connect(peer_b.db) as db_b:
            check("на каждом peer ровно один workspace", db_a.execute("SELECT count(*) FROM workspaces").fetchone()[0] == 1
                  and db_b.execute("SELECT count(*) FROM workspaces").fetchone()[0] == 1)
            blobs_a = {row[0] for row in db_a.execute("SELECT hash FROM content_blobs")}
            blobs_b = {row[0] for row in db_b.execute("SELECT hash FROM content_blobs")}
            check("full peer получает свой chat CAS и не получает чужой",
                  chat_files[0] in blobs_a and chat_files[1] not in blobs_a
                  and chat_files[1] in blobs_b and chat_files[0] not in blobs_b,
                  f"A={blobs_a} B={blobs_b}")
    finally:
        if peer_a is not None:
            peer_a.stop()
        if peer_b is not None:
            peer_b.stop()
        node.stop()

    print("\n===== CAPABILITY HTTP ИТОГ =====")
    print("failed:", len(failures))
    for failure in failures:
        print(" -", failure)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
