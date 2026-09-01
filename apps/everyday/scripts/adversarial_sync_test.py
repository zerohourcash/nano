"""Атаки, повторы, конфликты и полное восстановление sync-журнала."""

from __future__ import annotations

import copy
import json
import urllib.request
from http.cookiejar import CookieJar

from device_test_signing import DeviceSigner
from sync_test import (
    OWNER_PASSWORD,
    OWNER_PHONE,
    TOKEN,
    Node,
    check,
    failures,
    free_port,
    item_named,
    wait_for,
)


def journal(node: Node) -> dict:
    request = urllib.request.Request(f"{node.base}/sync/journal")
    request.add_header("authorization", f"Bearer {TOKEN}")
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.loads(response.read().decode())


def submit(node: Node, payload: dict) -> dict:
    request = urllib.request.Request(
        f"{node.base}/sync/journal",
        data=json.dumps(payload).encode(),
        method="POST",
    )
    request.add_header("authorization", f"Bearer {TOKEN}")
    request.add_header("content-type", "application/json")
    with urllib.request.urlopen(request, timeout=10) as response:
        return json.loads(response.read().decode())


def new_session(node: Node, label: str) -> None:
    node.cj = CookieJar()
    node.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(node.cj))
    node.signer = DeviceSigner(label)
    node.device_registered = False


def main() -> int:
    failures.clear()
    common = {"MESHKEEPER_SYNC_TOKEN": TOKEN, "MESHKEEPER_SYNC_INTERVAL": "5"}
    source = Node("attack-source", free_port(), common)
    target = Node("attack-target", free_port(), common)
    recovery = Node("attack-recovery", free_port(), common)
    try:
        check("три чистых узла запущены", source.wait_ready() and target.wait_ready() and recovery.wait_ready())
        owner = source.call(
            "auth.register",
            {
                "fullName": "Владелец Red Team",
                "phone": OWNER_PHONE,
                "password": OWNER_PASSWORD,
                "workspaceName": "Испытательный объект",
            },
        )
        ws = source.call("meta.workspaces", None, mutation=False)[0]["id"]
        tool = source.call("items.create", {"workspaceId": ws, "title": "Контрольная дрель"})
        source.call(
            "transfers.take",
            {"itemId": tool["id"], "dueAt": "2026-12-01T12:00:00.000Z"},
        )
        message = source.call(
            "chat.send", {"workspaceId": ws, "text": "Контрольное подписанное сообщение"}
        )
        signed = journal(source)
        check(
            "snapshot содержит подпись, историю и текущее состояние",
            bool(signed.get("journalHash"))
            and bool(signed.get("journalSignature"))
            and len(signed.get("history", [])) >= 3
            and signed["items"][0].get("statusSlug") == "in-work",
        )

        first = submit(target, signed)
        second = submit(target, signed)
        check(
            "двойная отправка принята идемпотентно",
            first.get("ok") is True
            and second.get("ok") is True
            and second.get("duplicate") is True,
            second,
        )
        check(
            "повтор не размножил операции и чат",
            len(journal(target).get("history", [])) == len(signed.get("history", []))
            and len(journal(target).get("messages", [])) == 1,
        )

        attacks: list[tuple[str, dict]] = []
        forged_state = copy.deepcopy(signed)
        forged_state["items"][0]["title"] = "ПОДМЕНЕНО АТАКУЮЩИМ"
        attacks.append(("подмена текущего состояния", forged_state))
        forged_history = copy.deepcopy(signed)
        forged_history["history"][-1]["comment"] = "поддельное содержание"
        attacks.append(("подмена ledger-события", forged_history))
        forged_membership = copy.deepcopy(signed)
        forged_membership["memberships"] = []
        attacks.append(("удаление прав и участников", forged_membership))
        missing_signature = copy.deepcopy(signed)
        missing_signature.pop("journalSignature", None)
        attacks.append(("удаление подписи snapshot", missing_signature))
        broken_signature = copy.deepcopy(signed)
        broken_signature["journalSignature"] = "AAAA"
        attacks.append(("фальсификация Ed25519-подписи", broken_signature))
        for label, payload in attacks:
            result = submit(target, payload)
            check(label + " отклонена", result.get("ok") is False, str(result)[:220])
        check(
            "серия атак не изменила принятую карточку",
            journal(target)["items"][0]["title"] == "Контрольная дрель",
        )

        # Чистое устройство получает одним проверяемым снимком и полную
        # историю, и вычисленное актуальное состояние.
        restored = submit(recovery, signed)
        login = recovery.call("auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD})
        recovery_ws = recovery.call("meta.workspaces", None, mutation=False)[0]["id"]
        restored_tool = item_named(recovery, recovery_ws, "Контрольная дрель") or {}
        restored_history = recovery.call("history.all", {"workspaceId": recovery_ws}, mutation=False)
        restored_chat = recovery.call("chat.list", {"workspaceId": recovery_ws}, mutation=False)
        restored_audit = recovery.call("sync.audit", None, mutation=False)
        check("чистое устройство приняло проверенный snapshot", restored.get("ok") is True)
        check("владелец может войти на восстановленном устройстве офлайн", isinstance(login, dict) and "id" in login)
        check(
            "восстановлено текущее состояние выдачи",
            restored_tool.get("status", {}).get("slug") == "in-work"
            and restored_tool.get("dueAt") == "2026-12-01T12:00:00.000Z",
            str(restored_tool)[:240],
        )
        check(
            "восстановлена вся история и связанный чат",
            isinstance(restored_history, list)
            and len(restored_history) == len(signed.get("history", []))
            and any(row.get("guid") == message.get("guid") for row in restored_chat),
        )
        check(
            "восстановленная нода сама доказывает целостность и полноту связей",
            restored_audit.get("healthy") is True
            and restored_audit.get("membershipVerified") is True
            and restored_audit.get("counts", {}).get("membershipVersions", 0) >= 1
            and restored_audit.get("ledgerVerified") == len(signed.get("history", []))
            and restored_audit.get("counts", {}).get("history") == len(signed.get("history", []))
            and restored_audit.get("orphanHistory") == 0
            and restored_audit.get("missingGuids") == 0
            and bool(restored_audit.get("snapshotHash")),
            str(restored_audit)[:300],
        )

        # Replay старого, но корректно подписанного snapshot не должен откатить
        # состояние после того, как пришёл более новый журнал.
        old = signed
        source.call("transfers.returnItem", {"itemId": tool["id"]})
        newest = journal(source)
        submit(target, newest)
        rollback = submit(target, old)
        target.call("auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD})
        target_ws = target.call("meta.workspaces", None, mutation=False)[0]["id"]
        check(
            "replay старого snapshot криптографически отклонён до изменения состояния",
            rollback.get("ok") is False
            and "rollback/replay" in rollback.get("error", "")
            and (item_named(target, target_ws, "Контрольная дрель") or {}).get("status", {}).get("slug")
            == "in-stock",
            rollback,
        )
        check(
            "после replay сохранена новая история без дублей",
            len(journal(target).get("history", [])) == len(newest.get("history", [])),
        )

        # Два человека получают одну и ту же исходную карточку, расходятся
        # офлайн и независимо подписывают выдачу одного инструмента.
        invite = source.call(
            "admin.workspaces.createInvite",
            {"workspaceId": ws, "role": "member", "maxUses": 1},
        )
        owner_session = (source.cj, source.opener, source.signer, source.device_registered)
        new_session(source, "conflict-member-registration")
        member_phone = "+7 900 777-66-55"
        member_password = "MemberConflict123"
        member = source.call(
            "auth.joinRegister",
            {
                "token": invite["token"],
                "fullName": "Второй кладовщик",
                "phone": member_phone,
                "password": member_password,
            },
        )
        source.cj, source.opener, source.signer, source.device_registered = owner_session
        check("создан второй авторизованный участник", isinstance(member, dict) and "id" in member)

        conflict_tool = source.call("items.create", {"workspaceId": ws, "title": "Конфликтная пила"})
        submit(target, journal(source))
        new_session(target, "conflict-member-device")
        target_login = target.call(
            "auth.login", {"phone": member_phone, "password": member_password}
        )
        target_ws = target.call("meta.workspaces", None, mutation=False)[0]["id"]
        target_tool = item_named(target, target_ws, "Конфликтная пила") or {}
        check("второй участник вошёл на изолированном узле", isinstance(target_login, dict) and "id" in target_login)

        left_take = source.call("transfers.take", {"itemId": conflict_tool["id"]})
        right_take = target.call("transfers.take", {"itemId": target_tool.get("id")})
        check(
            "две изолированные ноды независимо подписали выдачу",
            left_take.get("status", {}).get("slug") == "in-work"
            and right_take.get("status", {}).get("slug") == "in-work"
            and left_take.get("responsible", {}).get("fullName")
            != right_take.get("responsible", {}).get("fullName"),
        )
        conflict_import = submit(source, journal(target))
        conflicts = source.call("sync.conflicts", None, mutation=False)
        source_conflict_tool = item_named(source, ws, "Конфликтная пила") or {}
        check(
            "двойная офлайн-выдача обнаружена и помещена в карантин",
            conflict_import.get("conflicts", 0) >= 1
            and any(row.get("status") == "open" for row in conflicts)
            and source_conflict_tool.get("status", {}).get("slug") == "needs-check"
            and source_conflict_tool.get("responsibleUserId") is None,
            str(conflicts)[:300],
        )
        submit(target, journal(source))
        converged_tool = item_named(target, target_ws, "Конфликтная пила") or {}
        converged_history = target.call(
            "history.all", {"workspaceId": target_ws}, mutation=False
        )
        check(
            "подписанный факт конфликта разошёлся и выровнял состояние",
            converged_tool.get("status", {}).get("slug") == "needs-check"
            and converged_tool.get("responsibleUserId") is None
            and any(row.get("type") == "conflict_detected" for row in converged_history),
            str(converged_tool)[:240],
        )

        # Wiki-летопись не перезаписывает одну офлайн-версию другой: обе
        # подписанные дочерние ревизии остаются в DAG, а одинаковый hash-order
        # даёт всем узлам один текущий head и явный признак конфликта.
        base_page = source.call(
            "knowledge.save",
            {
                "workspaceId": ws,
                "slug": "safety/offline",
                "title": "Техника безопасности",
                "content": "Исходная согласованная инструкция",
            },
        )
        submit(target, journal(source))
        base_revision = base_page["savedRevisionGuid"]
        left_page = source.call(
            "knowledge.save",
            {
                "workspaceId": ws,
                "slug": "safety/offline",
                "title": "Техника безопасности",
                "content": "Изменение начальника на левой офлайн-ноде",
                "parentRevisionGuid": base_revision,
            },
        )
        right_page = target.call(
            "knowledge.save",
            {
                "workspaceId": target_ws,
                "slug": "safety/offline",
                "title": "Техника безопасности",
                "content": "Изменение кладовщика на правой офлайн-ноде",
                "parentRevisionGuid": base_revision,
            },
        )
        check(
            "две офлайн-ветки базы знаний независимо подписаны",
            bool(left_page.get("savedRevisionHash"))
            and bool(right_page.get("savedRevisionHash")),
        )
        submit(source, journal(target))
        submit(target, journal(source))
        submit(source, journal(target))
        source_page = source.call(
            "knowledge.bySlug", {"workspaceId": ws, "slug": "safety/offline"}, mutation=False
        )
        target_page = target.call(
            "knowledge.bySlug",
            {"workspaceId": target_ws, "slug": "safety/offline"},
            mutation=False,
        )
        check(
            "DAG сохранил обе ветки и детерминированно сошёлся",
            source_page.get("hasConflict") is True
            and target_page.get("hasConflict") is True
            and source_page.get("headGuids") == target_page.get("headGuids")
            and source_page.get("currentRevisionGuid")
            == target_page.get("currentRevisionGuid")
            and len(source_page.get("headGuids", [])) == 2,
            f"source={source_page} target={target_page}",
        )

        # Двойная трата Bit одним владельцем с двух офлайн-устройств.
        minted = source.call(
            "bit.mint",
            {"workspaceId": ws, "recipientUserId": owner["id"], "amount": 100, "memo": "Тестовый фонд"},
        )
        submit(target, journal(source))
        new_session(target, "conflict-owner-bit-device")
        target_owner = target.call(
            "auth.login", {"phone": OWNER_PHONE, "password": OWNER_PASSWORD}
        )
        target_people = target.call(
            "admin.users.list", {"workspaceId": target_ws}, mutation=False
        )
        target_member = next(row for row in target_people if row.get("fullName") == "Второй кладовщик")
        check("100 Bit доступны владельцу на двух изолированных нодах", minted.get("status") == "posted" and target_owner.get("id"))
        left_bit = source.call(
            "bit.transfer",
            {"workspaceId": ws, "recipientUserId": member["id"], "amount": 80, "memo": "Левая офлайн-ветвь"},
        )
        right_bit = target.call(
            "bit.transfer",
            {"workspaceId": target_ws, "recipientUserId": target_member["id"], "amount": 80, "memo": "Правая офлайн-ветвь"},
        )
        check("две конкурирующие траты локально приняты", left_bit.get("status") == "posted" and right_bit.get("status") == "posted")
        submit(source, journal(target))
        submit(target, journal(source))
        submit(source, journal(target))
        source_txs = source.call("bit.transactions", {"workspaceId": ws}, mutation=False)
        target_txs = target.call("bit.transactions", {"workspaceId": target_ws}, mutation=False)
        source_owner_balance = source.call("bit.balance", {"workspaceId": ws}, mutation=False)["balance"]
        source_member_balance = source.call("bit.balance", {"workspaceId": ws, "userId": member["id"]}, mutation=False)["balance"]
        target_owner_balance = target.call("bit.balance", {"workspaceId": target_ws}, mutation=False)["balance"]
        target_member_balance = target.call("bit.balance", {"workspaceId": target_ws, "userId": target_member["id"]}, mutation=False)["balance"]
        statuses_source = sorted(row["status"] for row in source_txs if row["kind"] == "transfer")
        statuses_target = sorted(row["status"] for row in target_txs if row["kind"] == "transfer")
        check(
            "двойная трата Bit детерминированно разрешена на обеих нодах",
            statuses_source == ["conflict", "posted"]
            and statuses_target == statuses_source
            and (source_owner_balance, source_member_balance) == (20, 80)
            and (target_owner_balance, target_member_balance) == (20, 80),
            f"source={statuses_source}/{source_owner_balance}/{source_member_balance} target={statuses_target}/{target_owner_balance}/{target_member_balance}",
        )

        # Один администратор меняет роль, а второй в той же исходной
        # версии отзывает членство. При равной Lamport-revision tombstone
        # всегда сильнее active-ветки, независимо от порядка доставки.
        left_role = source.call(
            "admin.users.update",
            {
                "workspaceId": ws,
                "id": member["id"],
                "organizationRole": "Аудитор офлайн",
                "personnelNumber": "LEFT-ROLE",
            },
        )
        right_revoke = target.call(
            "admin.users.remove",
            {"workspaceId": target_ws, "id": target_member["id"]},
        )
        check(
            "изолированные role-update и revoke локально приняты",
            left_role.get("id") == member["id"]
            and right_revoke.get("ok") is True,
            f"left={left_role} right={right_revoke}",
        )
        left_membership_journal = journal(source)
        right_membership_journal = journal(target)
        submit(source, right_membership_journal)
        submit(target, left_membership_journal)
        submit(source, journal(target))
        source_people = source.call(
            "admin.users.list", {"workspaceId": ws}, mutation=False
        )
        target_people = target.call(
            "admin.users.list", {"workspaceId": target_ws}, mutation=False
        )
        target_member_guid = target_member.get("guid")
        source_membership = next(
            row
            for row in journal(source).get("memberships", [])
            if row.get("userGuid") == target_member_guid
        )
        target_membership = next(
            row
            for row in journal(target).get("memberships", [])
            if row.get("userGuid") == target_member_guid
        )
        check(
            "concurrent revoke побеждает role-update и tombstone сходится",
            all(row.get("fullName") != "Второй кладовщик" for row in source_people)
            and all(row.get("fullName") != "Второй кладовщик" for row in target_people)
            and source_membership.get("active") is False
            and target_membership.get("active") is False
            and source_membership.get("revision") == target_membership.get("revision") == 2
            and source_membership.get("versionHash") == target_membership.get("versionHash"),
            f"source={source_membership} target={target_membership}",
        )
    finally:
        source.stop()
        target.stop()
        recovery.stop()

    print("\n===== ADVERSARIAL SYNC ИТОГ =====")
    print("failed:", len(failures))
    for failure in failures:
        print(" -", failure)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
