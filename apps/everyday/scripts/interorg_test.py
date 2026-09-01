"""Two independent organizations exchange an opaque transaction after a partition."""

from __future__ import annotations

import json
import sqlite3
import time
import urllib.error
import urllib.request
import uuid

from sync_test import Node, free_port, wait_for


def unauthorized_journal(node: Node) -> int:
    try:
        urllib.request.urlopen(f"{node.base}/sync/journal", timeout=3)
        return 200
    except urllib.error.HTTPError as error:
        return error.code


def item_titles(node: Node, workspace_id: int) -> list[str]:
    value = node.call("items.list", {"workspaceId": workspace_id}, mutation=False)
    rows = value.get("rows", []) if isinstance(value, dict) else value
    return [item["title"] for item in rows] if isinstance(rows, list) else []


def main() -> int:
    failures: list[str] = []

    def check(label: str, condition: bool, detail=""):
        print(f"[{'OK  ' if condition else 'FAIL'}] {label} {detail}")
        if not condition:
            failures.append(label)

    a = Node("interorg-a", free_port(), {})
    b = Node("interorg-b", free_port(), {})
    try:
        check("две независимые ноды запущены без общей capability", a.wait_ready() and b.wait_ready())
        a.call("auth.register", {
            "fullName": "Владелец А", "phone": "+7 900 810-00-01",
            "password": "InterorgOwnerA123", "workspaceName": "Организация А",
        })
        b.call("auth.register", {
            "fullName": "Владелец Б", "phone": "+7 900 810-00-02",
            "password": "InterorgOwnerB123", "workspaceName": "Организация Б",
        })
        ws_a = a.call("meta.workspaces", None, mutation=False)[0]
        ws_b = b.call("meta.workspaces", None, mutation=False)[0]
        identity_a = a.call("interorg.ensureIdentity", {"workspaceId": ws_a["id"]})
        identity_b = b.call("interorg.ensureIdentity", {"workspaceId": ws_b["id"]})
        check("обе организации получили разные gateway-адреса",
              len(identity_a.get("destination", "")) == 64
              and len(identity_b.get("destination", "")) == 64
              and identity_a["destination"] != identity_b["destination"])

        contact_b = a.call("interorg.trustContact", {
            "workspaceId": ws_a["id"], "name": "Организация Б",
            "remoteWorkspaceGuid": ws_b["guid"],
            "encryptionKey": identity_b["publicKey"], "signingKey": identity_b["signingKey"],
        })
        contact_a = b.call("interorg.trustContact", {
            "workspaceId": ws_b["id"], "name": "Организация А",
            "remoteWorkspaceGuid": ws_a["guid"],
            "encryptionKey": identity_a["publicKey"], "signingKey": identity_a["signingKey"],
        })
        check("контрагенты явно добавлены в разные доверенные каталоги",
              contact_b.get("remoteWorkspaceGuid") == ws_b["guid"])

        a.call("items.create", {"workspaceId": ws_a["id"], "title": "Только А"})
        b.call("items.create", {"workspaceId": ws_b["id"], "title": "Только Б"})
        check("обычные данные организаций не пересекаются",
              item_titles(a, ws_a["id"]) == ["Только А"]
              and item_titles(b, ws_b["id"]) == ["Только Б"])
        check("без общей capability scoped journal закрыт", unauthorized_journal(a) == 401 and unauthorized_journal(b) == 401)

        # Partition: B is physically absent. A queues a device-signed transaction.
        b.stop(cleanup=False)
        a.stop(cleanup=False)
        a.env["MESHKEEPER_RELAY_PEERS"] = b.base
        a.restart()
        check("A перезапущена с недоступным relay и сохранила историю", a.wait_ready())
        transaction_id = str(uuid.uuid4())
        text = "Офлайн-заказ 17 Bit, смена 4"
        sent = a.call("interorg.send", {
            "workspaceId": ws_a["id"], "contactGuid": contact_b["guid"],
            "transactionId": transaction_id, "kind": "invoice.offer", "body": {"text": text},
        })
        check("транзакция подписана и осталась в offline-очереди",
              sent.get("queued") is True and len(sent.get("ledgerHash", "")) == 64, str(sent))
        time.sleep(2.5)
        with sqlite3.connect(a.db) as database:
            raw = database.execute("SELECT envelope_json FROM interorg_envelopes WHERE id=?", (sent["envelopeId"],)).fetchone()[0]
        check("relay-конверт не раскрывает текст и GUID организаций",
              text not in raw and ws_a["guid"] not in raw and ws_b["guid"] not in raw)

        # Connectivity returns. No journal capability is introduced: only opaque gossip.
        b.env["MESHKEEPER_RELAY_PEERS"] = a.base
        b.restart()
        check("B вернулась после разделения", b.wait_ready())

        def inbox_b():
            value = b.call("interorg.inbox", {"workspaceId": ws_b["id"]}, mutation=False)
            return value if isinstance(value, list) else []

        check("B автоматически получила и расшифровала адресную транзакцию",
              wait_for(lambda: len(inbox_b()) == 1, timeout=20), str(inbox_b()))
        incoming = inbox_b()[0]
        check("получатель проверил автора, transaction ID и точный текст",
              incoming["transactionId"] == transaction_id
              and incoming["contact"]["remoteWorkspaceGuid"] == ws_a["guid"]
              and incoming["body"]["text"] == text, str(incoming))
        accepted = b.call("interorg.accept", {
            "workspaceId": ws_b["id"], "envelopeId": incoming["envelopeId"],
        })
        duplicate = b.call("interorg.accept", {
            "workspaceId": ws_b["id"], "envelopeId": incoming["envelopeId"],
        })
        check("принятие записано в Ledger и повтор идемпотентен",
              len(accepted.get("ledgerHash", "")) == 64 and duplicate.get("duplicate") is True)
        with sqlite3.connect(b.db) as database:
            accepted_rows = database.execute(
                "SELECT COUNT(*) FROM history_entries WHERE type='interorg_accept' AND to_label=?",
                (transaction_id,),
            ).fetchone()[0]
            foreign_items = database.execute("SELECT COUNT(*) FROM items WHERE title='Только А'").fetchone()[0]
        check("принятие существует ровно один раз, чужой склад не реплицирован",
              accepted_rows == 1 and foreign_items == 0, f"accept={accepted_rows} foreignItems={foreign_items}")

        revoked = b.call("interorg.revokeContact", {
            "workspaceId": ws_b["id"], "guid": contact_a["guid"],
        })
        blocked_tx = str(uuid.uuid4())
        a.call("interorg.send", {
            "workspaceId": ws_a["id"], "contactGuid": contact_b["guid"],
            "transactionId": blocked_tx, "kind": "message.notice",
            "body": {"text": "сообщение после отзыва"},
        })
        time.sleep(4)
        check("после signed revoke новый конверт не становится входящей транзакцией",
              len(revoked.get("ledgerHash", "")) == 64
              and all(row["transactionId"] != blocked_tx for row in inbox_b()))
    finally:
        a.stop()
        b.stop()

    print("\n===== INTERORG ИТОГ =====")
    print(f"failed: {len(failures)}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
