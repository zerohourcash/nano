"""Деградационный тест автономной P2P-сети из трёх Everyday-узлов."""

from __future__ import annotations

import json
import base64
import sqlite3
import urllib.request

from sync_test import (
    OWNER_PASSWORD,
    OWNER_PHONE,
    TOKEN,
    Node,
    check,
    failures,
    free_port,
    wait_for,
)


def journal(node: Node) -> dict:
    request = urllib.request.Request(f"{node.base}/sync/journal")
    request.add_header("authorization", f"Bearer {TOKEN}")
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.loads(response.read().decode())


def peer(node: Node, url: str) -> dict | None:
    status = node.call("sync.status", None, mutation=False)
    rows = status.get("peers", []) if isinstance(status, dict) else []
    return next((row for row in rows if row.get("url") == url), None)


def item_named(node: Node, workspace_id: int, title: str) -> dict | None:
    rows = node.call("reports.allItems", {"workspaceId": workspace_id}, mutation=False)
    return next((row for row in rows if row.get("title") == title), None) if isinstance(rows, list) else None


def main() -> int:
    failures.clear()
    a_port, b_port, c_port = free_port(), free_port(), free_port()
    common = {"MESHKEEPER_SYNC_TOKEN": TOKEN, "MESHKEEPER_SYNC_INTERVAL": "5"}
    a = Node("mesh-a", a_port, {**common, "MESHKEEPER_CONTENT_MODE": "full", "MESHKEEPER_ADVERTISE_URL": f"http://127.0.0.1:{a_port}"})
    b = Node("mesh-b", b_port, {**common, "MESHKEEPER_CONTENT_MODE": "metadata", "MESHKEEPER_ADVERTISE_URL": f"http://127.0.0.1:{b_port}"})
    c: Node | None = None
    try:
        check("узлы A и B запущены", a.wait_ready() and b.wait_ready())
        owner = a.call(
            "auth.register",
            {
                "fullName": "Владелец Mesh",
                "phone": OWNER_PHONE,
                "password": OWNER_PASSWORD,
                "workspaceName": "Автономная бригада",
            },
        )
        check("организация создана на A", isinstance(owner, dict) and "id" in owner)

        b_url = b.base
        added = a.call("sync.addPeer", {"url": b_url, "name": "Узел B"})
        check("B добавлен как исполняемый peer", added.get("ok") is True, str(added))
        a.call("sync.pullNow", {})
        check(
            "A передал организацию B без upstream",
            wait_for(lambda: len(journal(b).get("workspaces", [])) == 1),
        )
        login_b = b.call("auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD})
        check("владелец входит на B офлайн", isinstance(login_b, dict) and "id" in login_b)
        check("B узнал A из подписанного обмена", peer(b, a.base) is not None)

        dead_c_url = f"http://127.0.0.1:{c_port}"
        b.call("sync.addPeer", {"url": dead_c_url, "name": "Узел C"})
        b.call("sync.pullNow", {})
        check(
            "недоступный C отмечен ошибкой, B продолжает работать",
            wait_for(lambda: bool((peer(b, dead_c_url) or {}).get("lastError")), timeout=15),
            str(peer(b, dead_c_url)),
        )

        c = Node("mesh-c", c_port, {**common, "MESHKEEPER_ADVERTISE_URL": dead_c_url})
        check("C появился после разрыва", c.wait_ready())
        b.call("sync.pullNow", {})
        check(
            "связь B↔C восстановилась и данные сошлись",
            wait_for(lambda: len(journal(c).get("workspaces", [])) == 1),
        )
        check(
            "ошибка C очищена после успешного обмена",
            wait_for(lambda: not (peer(b, dead_c_url) or {}).get("lastError")),
            str(peer(b, dead_c_url)),
        )
        login_c = c.call("auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD})
        check("владелец входит на C офлайн", isinstance(login_c, dict) and "id" in login_c)

        ws_c = c.call("meta.workspaces", None, mutation=False)[0]["id"]
        message = c.call(
            "chat.send",
            {"workspaceId": ws_c, "text": "Сообщение C через локальную mesh"},
        )
        check(
            "сообщение C подписано устройством и ledger",
            message.get("ledgerVerified") is True and bool(message.get("ledgerHash")),
            str(message)[:200],
        )
        c.call("sync.pullNow", {})
        check(
            "чат C доставлен через B на A ровно один раз",
            wait_for(
                lambda: len(
                    [
                        row
                        for row in a.call("chat.list", {"workspaceId": 1}, mutation=False)
                        if row.get("guid") == message.get("guid") and row.get("ledgerVerified") is True
                    ]
                )
                == 1
            ),
        )

        mesh_photo = "data:image/webp;base64," + base64.b64encode(bytes([17]) * 90_000).decode()
        created_c = c.call(
            "items.create",
            {
                "workspaceId": ws_c,
                "title": "Рация с узла C",
                "photos": [{"url": mesh_photo, "thumbUrl": mesh_photo}],
            },
        )
        check("операция создана на C", isinstance(created_c, dict) and "id" in created_c)
        c.call("sync.pullNow", {})
        check(
            "операция C дошла через B до A",
            wait_for(lambda: item_named(a, 1, "Рация с узла C") is not None),
        )
        check(
            "CAS-фото прошло транзитивно C→B→A без интернета",
            wait_for(
                lambda: ((item_named(a, 1, "Рация с узла C") or {}).get("photos") or [{}])[0].get("url")
                == mesh_photo,
                timeout=30,
            ),
        )
        with sqlite3.connect(b.db) as db:
            b_blobs = db.execute("SELECT count(*) FROM content_blobs").fetchone()[0]
        check(
            "metadata-ретранслятор передал каталог, не сохраняя файл",
            b_blobs == 0,
            f"content_blobs={b_blobs}",
        )

        taken = c.call(
            "transfers.take",
            {"itemId": created_c["id"], "dueAt": "2026-09-30T12:00:00.000Z"},
        )
        check(
            "C подписал локальную выдачу со статусом «В работе»",
            taken.get("responsibleUserId") is not None
            and taken.get("status", {}).get("slug") == "in-work",
            str(taken)[:240],
        )
        c.call("sync.pullNow", {})
        check(
            "статус и срок выдачи сошлись на A через B",
            wait_for(
                lambda: (item_named(a, 1, "Рация с узла C") or {}).get("dueAt")
                == "2026-09-30T12:00:00.000Z"
                and (item_named(a, 1, "Рация с узла C") or {}).get("status", {}).get("slug")
                == "in-work"
            ),
            str(item_named(a, 1, "Рация с узла C"))[:300],
        )
        radio_a = item_named(a, 1, "Рация с узла C") or {}
        returned = a.call("transfers.returnItem", {"itemId": radio_a.get("id")})
        check(
            "A подписал возврат на склад",
            returned.get("responsibleUserId") is None
            and returned.get("status", {}).get("slug") == "in-stock",
            str(returned)[:240],
        )
        a.call("sync.pullNow", {})
        check(
            "возврат очистил ответственного и срок на C",
            wait_for(
                lambda: (item_named(c, ws_c, "Рация с узла C") or {}).get("responsibleUserId") is None
                and (item_named(c, ws_c, "Рация с узла C") or {}).get("dueAt") is None
                and (item_named(c, ws_c, "Рация с узла C") or {}).get("status", {}).get("slug")
                == "in-stock"
            ),
            str(item_named(c, ws_c, "Рация с узла C"))[:300],
        )

        # Полная изоляция B: обе соседние ноды недоступны, но локальные
        # транзакции должны приниматься и пережить несколько неудачных тиков.
        a.stop(cleanup=False)
        c.stop(cleanup=False)
        ws_b = b.call("meta.workspaces", None, mutation=False)[0]["id"]
        created_b = b.call("items.create", {"workspaceId": ws_b, "title": "Фонарь после отключения A"})
        check("B принимает операции при полной потере сети", isinstance(created_b, dict) and "id" in created_b)
        delayed_take = b.call(
            "transfers.take",
            {"itemId": created_b["id"], "dueAt": "2026-10-31T12:00:00.000Z"},
        )
        check(
            "отложенная выдача подписана и сохранена локально",
            delayed_take.get("status", {}).get("slug") == "in-work",
            str(delayed_take)[:240],
        )
        delayed_message = b.call(
            "chat.send",
            {"workspaceId": ws_b, "text": "Отложенное сообщение из полной изоляции"},
        )
        check(
            "отложенное сообщение подписано и сохранено локально",
            delayed_message.get("ledgerVerified") is True,
            str(delayed_message)[:240],
        )
        check(
            "неудачные доставки сохраняются для повтора",
            wait_for(
                lambda: bool((peer(b, a.base) or {}).get("lastError"))
                and bool((peer(b, dead_c_url) or {}).get("lastError")),
                timeout=15,
            ),
        )

        # Возвращаем обе ноды с прежними БД. Никакого pullNow: фоновый цикл
        # обязан сам повторить store-and-forward доставку.
        a.restart()
        c.restart()
        check("A и C восстановлены с прежней историей", a.wait_ready() and c.wait_ready())
        check(
            "отложенная выдача автоматически доставлена A и C",
            wait_for(
                lambda: (item_named(a, 1, "Фонарь после отключения A") or {}).get("dueAt")
                == "2026-10-31T12:00:00.000Z"
                and (item_named(c, ws_c, "Фонарь после отключения A") or {}).get("dueAt")
                == "2026-10-31T12:00:00.000Z",
                timeout=30,
            ),
        )
        check(
            "отложенный чат доставлен ровно один раз на каждую ноду",
            wait_for(
                lambda: len(
                    [
                        row
                        for row in a.call("chat.list", {"workspaceId": 1}, mutation=False)
                        if row.get("guid") == delayed_message.get("guid")
                    ]
                )
                == 1
                and len(
                    [
                        row
                        for row in c.call("chat.list", {"workspaceId": ws_c}, mutation=False)
                        if row.get("guid") == delayed_message.get("guid")
                    ]
                )
                == 1,
                timeout=30,
            ),
        )
    finally:
        if a.proc.poll() is None:
            a.stop()
        b.stop()
        if c is not None:
            c.stop()

    print("\n===== MESH ИТОГ =====")
    print("failed:", len(failures))
    for failure in failures:
        print(" -", failure)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
