"""Two independent organizations exchange an opaque transaction after a partition."""

from __future__ import annotations

import json
import base64
import hashlib
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

        # Connectivity returns. No journal capability is introduced. The same
        # bounded bundle used by Android BLE is carried explicitly here.
        b.restart()
        check("B вернулась после разделения", b.wait_ready())

        def carry(source: Node, workspace_id: int, target: Node):
            bundle = source.call(
                "interorg.gossip", {"workspaceId": workspace_id}, mutation=False)
            check("transport выдаёт bounded opaque interorg gossip",
                  bundle.get("format") == "everyday-interorg-gossip"
                  and bundle.get("version") == 1
                  and 0 < len(bundle.get("envelopes", [])) <= 32)
            return target.call("interorg.importGossip", {"bundle": bundle})

        first_delivery = carry(a, ws_a["id"], b)
        check("BLE-совместимый gossip атомарно проверен и доставлен",
              first_delivery.get("stored", 0) >= 1
              and first_delivery.get("delivered", 0) >= 1,
              str(first_delivery))

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
        with sqlite3.connect(b.db) as database:
            original_body = database.execute(
                "SELECT body_json FROM interorg_inbox WHERE envelope_id=?",
                (incoming["envelopeId"],),
            ).fetchone()[0]
            database.execute(
                "UPDATE interorg_inbox SET body_json=? WHERE envelope_id=?",
                (json.dumps({"text": "локальная подмена", "amount": 999999}), incoming["envelopeId"]),
            )
            database.commit()
        forged_inbox_audit = b.call("sync.audit", None, mutation=False)
        check("полный аудит повторно открывает AEAD envelope и обнаруживает подмену inbox",
              forged_inbox_audit.get("healthy") is False
              and "signed envelope" in (forged_inbox_audit.get("interorgInboxError") or ""),
              str(forged_inbox_audit.get("interorgInboxError")))
        with sqlite3.connect(b.db) as database:
            database.execute(
                "UPDATE interorg_inbox SET body_json=? WHERE envelope_id=?",
                (original_body, incoming["envelopeId"]),
            )
            database.commit()
        restored_inbox_audit = b.call("sync.audit", None, mutation=False)
        check("после восстановления точного payload межорганизационный аудит снова здоров",
              restored_inbox_audit.get("healthy") is True
              and restored_inbox_audit.get("interorgInboxVerified") == 1
              and restored_inbox_audit.get("interorgInboxLegacy") == 0,
              str(restored_inbox_audit.get("interorgInboxError")))
        accepted = b.call("interorg.accept", {
            "workspaceId": ws_b["id"], "envelopeId": incoming["envelopeId"],
        })
        duplicate = b.call("interorg.accept", {
            "workspaceId": ws_b["id"], "envelopeId": incoming["envelopeId"],
        })
        check("принятие записано в Ledger и повтор идемпотентен",
              len(accepted.get("ledgerHash", "")) == 64
              and accepted.get("receiptQueued") is True
              and duplicate.get("duplicate") is True)
        receipt_delivery = carry(b, ws_b["id"], a)
        check("обратная квитанция перенесена тем же transport bundle",
              receipt_delivery.get("stored", 0) >= 1, str(receipt_delivery))

        def accepted_outbox_a():
            rows = a.call("interorg.outbox", {"workspaceId": ws_a["id"]}, mutation=False)
            return next((row for row in rows if row["transactionId"] == transaction_id
                         and row["status"] == "accepted"), None)

        check("зашифрованная квитанция прошла обратно через mesh и связалась с исходной транзакцией",
              wait_for(lambda: accepted_outbox_a() is not None, timeout=20),
              str(a.call("interorg.outbox", {"workspaceId": ws_a["id"]}, mutation=False)))
        receipt = accepted_outbox_a()
        check("отправитель получил hash летописи получателя, но не внутреннюю базу",
              receipt is not None
              and receipt["acceptanceLedgerHash"] == accepted["ledgerHash"]
              and len(receipt["receiptEnvelopeId"]) == 36
              and receipt["acceptanceProofVerified"] is True
              and receipt["acceptanceProof"]["type"] == "interorg_accept"
              and receipt["acceptanceProof"]["toLabel"] == transaction_id
              and receipt["acceptanceProof"]["requestPath"] == "/api/trpc/interorg.accept",
              str(receipt))
        audit = a.call("sync.audit", None, mutation=False)
        check("общий аудит повторно проверил межорганизационное device+node proof",
              audit.get("healthy") is True
              and audit.get("interorgReceiptsVerified", 0) >= 1,
              str(audit.get("interorgReceiptError")))
        with sqlite3.connect(a.db) as database:
            original_proof = database.execute(
                "SELECT acceptance_proof_json FROM interorg_outbox WHERE transaction_id=?",
                (transaction_id,),
            ).fetchone()[0]
            forged_proof = json.loads(original_proof)
            forged_proof["toLabel"] = str(uuid.uuid4())
            database.execute(
                "UPDATE interorg_outbox SET acceptance_proof_json=? WHERE transaction_id=?",
                (json.dumps(forged_proof), transaction_id),
            )
            database.commit()
        forged_audit = a.call("sync.audit", None, mutation=False)
        forged_outbox = a.call("interorg.outbox", {"workspaceId": ws_a["id"]}, mutation=False)
        check("локальная подмена сохранённой квитанции обнаружена при чтении и полном аудите",
              forged_audit.get("healthy") is False
              and "proof mismatch" in (forged_audit.get("interorgReceiptError") or "")
              and forged_outbox[0]["acceptanceProofVerified"] is False)
        with sqlite3.connect(a.db) as database:
            database.execute(
                "UPDATE interorg_outbox SET acceptance_proof_json=? WHERE transaction_id=?",
                (original_proof, transaction_id),
            )
            database.commit()
        with sqlite3.connect(b.db) as database:
            accepted_rows = database.execute(
                "SELECT COUNT(*) FROM history_entries WHERE type='interorg_accept' AND to_label=?",
                (transaction_id,),
            ).fetchone()[0]
            foreign_items = database.execute("SELECT COUNT(*) FROM items WHERE title='Только А'").fetchone()[0]
        check("принятие существует ровно один раз, чужой склад не реплицирован",
              accepted_rows == 1 and foreign_items == 0, f"accept={accepted_rows} foreignItems={foreign_items}")

        replay = a.call("interorg.send", {
            "workspaceId": ws_a["id"], "contactGuid": contact_b["guid"],
            "transactionId": transaction_id, "kind": "invoice.offer",
            "body": {"text": "подмена уже принятой транзакции", "amount": 999999},
        })
        carry(a, ws_a["id"], b)

        def replay_quarantined():
            with sqlite3.connect(b.db) as database:
                row = database.execute(
                    "SELECT delivered FROM interorg_envelopes WHERE id=?", (replay["envelopeId"],),
                ).fetchone()
                return row is not None and row[0] == 2

        check("новый envelope с прежним transaction ID помещён в карантин",
              wait_for(replay_quarantined, timeout=20)
              and len(inbox_b()) == 1
              and inbox_b()[0]["body"]["text"] == text)

        file_bytes = b"Everyday encrypted interorg attachment\x00private shift report"
        file_tx = str(uuid.uuid4())
        file_name = "shift-private.bin"
        file_body = {
            "name": file_name,
            "mime": "application/octet-stream",
            "size": len(file_bytes),
            "sha256": hashlib.sha256(file_bytes).hexdigest(),
            "dataBase64": base64.b64encode(file_bytes).decode("ascii"),
            "text": "Закрытое вложение смены",
        }
        file_sent = a.call("interorg.send", {
            "workspaceId": ws_a["id"], "contactGuid": contact_b["guid"],
            "transactionId": file_tx, "kind": "message.file", "body": file_body,
        })
        file_delivery = carry(a, ws_a["id"], b)
        check("файловый envelope принят через тот же BLE-совместимый gossip",
              file_delivery.get("stored", 0) >= 1, str(file_delivery))
        with sqlite3.connect(a.db) as database:
            file_wire = database.execute(
                "SELECT envelope_json FROM interorg_envelopes WHERE id=?",
                (file_sent["envelopeId"],),
            ).fetchone()[0]
        check("relay не видит имя, MIME и байты межорганизационного файла",
              file_name not in file_wire
              and file_body["dataBase64"] not in file_wire
              and file_body["mime"] not in file_wire)

        def received_file():
            return next((row for row in inbox_b() if row["transactionId"] == file_tx), None)

        check("небольшой файл доставлен после разрыва как подписанная адресная транзакция",
              wait_for(lambda: received_file() is not None, timeout=20), str(inbox_b()))
        received = received_file()
        check("получатель восстановил точные байты и SHA-256 вложения",
              received is not None
              and received["kind"] == "message.file"
              and base64.b64decode(received["body"]["dataBase64"]) == file_bytes
              and received["body"]["sha256"] == hashlib.sha256(file_bytes).hexdigest(),
              str(received))
        file_audit = b.call("sync.audit", None, mutation=False)
        check("полный аудит повторно проверил AEAD-конверт с файлом",
              file_audit.get("healthy") is True
              and file_audit.get("interorgInboxVerified", 0) == 2,
              str(file_audit.get("interorgInboxError")))

        revoked = b.call("interorg.revokeContact", {
            "workspaceId": ws_b["id"], "guid": contact_a["guid"],
        })
        blocked_tx = str(uuid.uuid4())
        a.call("interorg.send", {
            "workspaceId": ws_a["id"], "contactGuid": contact_b["guid"],
            "transactionId": blocked_tx, "kind": "message.notice",
            "body": {"text": "сообщение после отзыва"},
        })
        carry(a, ws_a["id"], b)
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
