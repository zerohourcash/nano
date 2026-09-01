"""Attack the signed internal chat through the real HTTP/device-proof path."""

from __future__ import annotations

import sqlite3

from sync_test import Node, free_port


def main() -> int:
    failures: list[str] = []

    def check(label: str, condition: bool, detail=""):
        print(f"[{'OK  ' if condition else 'FAIL'}] {label} {detail}")
        if not condition:
            failures.append(label)

    node = Node("spam-gate", free_port(), {})
    try:
        check("чистый узел запущен", node.wait_ready())
        owner = node.call("auth.register", {
            "fullName": "Антиспам Аудитор",
            "phone": "+7 900 830-00-01",
            "password": "SpamAuditOwner123",
            "workspaceName": "Антиспам",
        })
        workspace = node.call("meta.workspaces", None, mutation=False)[0]
        check("владелец зарегистрирован", isinstance(owner, dict) and owner.get("id") == 1)

        first = node.call("chat.send", {"workspaceId": workspace["id"], "text": "Сообщение 0"})
        duplicate = node.call("chat.send", {"workspaceId": workspace["id"], "text": "Сообщение 0"})
        check("быстрый подписанный дубль отклонён",
              len(first.get("ledgerHash", "")) == 64
              and duplicate.get("__err") == "Такое сообщение уже отправлено",
              str(duplicate))

        for index in range(1, 20):
            sent = node.call("chat.send", {
                "workspaceId": workspace["id"], "text": f"Сообщение {index}",
            })
            if len(sent.get("ledgerHash", "")) != 64:
                failures.append(f"сообщение {index} не принято")
                break
        flooded = node.call("chat.send", {
            "workspaceId": workspace["id"], "text": "Сообщение 20",
        })
        check("двадцать первое сообщение за минуту отклонено",
              flooded.get("__err") == "Слишком много сообщений: подождите минуту",
              str(flooded))

        oversized = node.call("chat.send", {
            "workspaceId": workspace["id"], "text": "я" * 4001,
        })
        check("сообщение сверх лимита символов отклонено до записи",
              oversized.get("__err") == "Сообщение длиннее 4000 символов",
              str(oversized))

        with sqlite3.connect(node.db) as database:
            messages = database.execute("SELECT COUNT(*) FROM chat_messages").fetchone()[0]
            events = database.execute(
                "SELECT COUNT(*) FROM history_entries WHERE type='chat_message'"
            ).fetchone()[0]
        check("все отказы атомарны: нет сообщений и Ledger-полутранзакций",
              (messages, events) == (20, 20), f"messages={messages} events={events}")

        node.stop(cleanup=False)
        node.restart()
        check("узел перезапущен на прежней offline-базе", node.wait_ready())
        persisted = node.call("chat.send", {
            "workspaceId": workspace["id"], "text": "Обход после restart",
        })
        check("restart процесса не сбрасывает persisted rate-limit",
              persisted.get("__err") == "Слишком много сообщений: подождите минуту",
              str(persisted))
    finally:
        node.stop()

    print("\n===== ANTISPAM ИТОГ =====")
    print(f"failed: {len(failures)}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
