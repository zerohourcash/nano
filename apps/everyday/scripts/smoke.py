"""Сценарии ТЗ через HTTP-API узла. Запускать через scripts/smoke_runner.py."""

import json, os, urllib.error, urllib.parse, urllib.request, http.cookiejar
from device_test_signing import CRITICAL, DeviceSigner

BASE = os.environ.get("MK_BASE", "http://127.0.0.1:8098")
ORIGIN = BASE

class Client:
    def __init__(self, name):
        self.name = name
        self.cj = http.cookiejar.CookieJar()
        self.op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.cj))
        self.signer = DeviceSigner(name)
        self.device_registered = False

    def call(self, proc, inp=None, mutation=True, raw=False, signed=True):
        url = f"{BASE}/api/trpc/{proc}?batch=1"
        body = json.dumps({"0": {"json": inp}}).encode()
        req = urllib.request.Request(url, data=body if mutation else None, method="POST" if mutation else "GET")
        req.add_header("content-type", "application/json")
        req.add_header("origin", ORIGIN)
        if mutation and proc in CRITICAL and signed:
            if not self.device_registered:
                enrolled = self.call("auth.registerDevice", {
                    "deviceId": self.signer.device_id,
                    "name": self.name,
                    "publicKey": self.signer.public_key,
                })
                if not isinstance(enrolled, dict) or enrolled.get("deviceId") != self.signer.device_id:
                    return {"__err": "device enrollment failed", "__detail": enrolled}
                self.device_registered = True
            for key, value in self.signer.headers(f"/api/trpc/{proc}", body).items():
                req.add_header(key, value)
        if not mutation:
            url = f"{BASE}/api/trpc/{proc}?batch=1&input=" + urllib.parse.quote(json.dumps({"0": {"json": inp}}))
            req = urllib.request.Request(url, method="GET")
            req.add_header("origin", ORIGIN)
        try:
            with self.op.open(req, timeout=20) as r:
                data = json.loads(r.read().decode())
        except urllib.error.HTTPError as e:
            return {"__http": e.code, "__body": e.read().decode()[:300]}
        if isinstance(data, list):
            data = data[0]
        if raw:
            return data
        if "error" in data:
            return {"__err": data["error"]["json"].get("message"), "__code": data["error"]["json"].get("data", {}).get("code")}
        return data["result"]["data"]["json"]


def show(label, v):
    t = json.dumps(v, ensure_ascii=False)
    if len(t) > 400:
        t = t[:400] + "…"
    print(f"{label}: {t}")


fails = []
def check(label, cond, detail=""):
    status = "OK  " if cond else "FAIL"
    if not cond:
        fails.append(label)
    print(f"[{status}] {label} {detail}")


owner = Client("owner")
print("== 1. Регистрация владельца ==")
r = owner.call("auth.register", {"fullName": "Дима Владелец", "phone": "+7 900 111-22-33", "password": "SuperSecret123", "workspaceName": "Объект Северный"})
show("register", r)
check("register owner", "id" in r)
uid_owner = r.get("id")
ws = owner.call("meta.workspaces", None, mutation=False)
show("workspaces", ws)
ws_id = ws[0]["id"] if isinstance(ws, list) and ws else None

print("\n== 2. auth.me / сессия ==")
me = owner.call("auth.me", None, mutation=False)
show("me", me)
check("session works", isinstance(me, dict) and me.get("id") == uid_owner)

anon = Client("anon")
me2 = anon.call("auth.me", None, mutation=False)
check("anonymous has no session", me2 in (None, {}) or "__err" in me2, json.dumps(me2, ensure_ascii=False)[:120])

print("\n== 3. Повторная открытая регистрация закрыта ==")
r2 = Client("x").call("auth.register", {"fullName": "Чужой", "phone": "+7 900 999-00-11", "password": "SuperSecret123"})
check("open registration closed after bootstrap", "__err" in r2, str(r2)[:120])

print("\n== 4. Справочники и создание инструмента ==")
dicts = owner.call("admin.dictionaries.list", {"workspaceId": ws_id, "kind": "categories"}, mutation=False)
show("categories", dicts)
cat = owner.call("admin.dictionaries.create", {"workspaceId": ws_id, "kind": "categories", "name": "Электроинструмент"})
show("cat create", cat)
storages = owner.call("admin.storages.list", {"workspaceId": ws_id}, mutation=False)
show("storages", storages)
st_id = storages[0]["id"] if isinstance(storages, list) and storages else None
nid = owner.call("items.nextInternalId", {"workspaceId": ws_id}, mutation=False)
show("nextInternalId", nid)
item = owner.call("items.create", {
    "workspaceId": ws_id, "title": "Перфоратор Bosch GBH", "internalId": nid if isinstance(nid, str) else "ВН-0001",
    "categoryId": cat.get("id") if isinstance(cat, dict) else None,
    "storageId": st_id, "serialNumber": "SN-777", "quantity": 1,
    "sourceSystem": "facekit", "externalId": "2877042",
    "metadata": {
        "ownerCompany": "ООО ФейсКИТ", "currency": "RUB",
        "labels": ["демо", "инструмент"], "isKit": False,
        "supplier": "Тестовый поставщик", "documentNumber": "СЧ-77",
    },
})
show("item create", item)
check("item created", isinstance(item, dict) and "id" in item)
check(
    "FaceKit metadata preserved",
    isinstance(item, dict)
    and item.get("sourceSystem") == "facekit"
    and item.get("externalId") == "2877042"
    and item.get("metadata", {}).get("documentNumber") == "СЧ-77",
    str(item.get("metadata") if isinstance(item, dict) else item)[:180],
)
item_id = item.get("id") if isinstance(item, dict) else None

lst = owner.call("items.list", {"workspaceId": ws_id}, mutation=False)
check("items.list returns the item", isinstance(lst, (list, dict)) and json.dumps(lst, ensure_ascii=False).find("Перфоратор") >= 0)

print("\n== 5. byCode (QR по внутреннему номеру) ==")
code = item.get("internalId") if isinstance(item, dict) else None
bc = owner.call("items.byCode", {"code": code}, mutation=False)
check("items.byCode finds item", isinstance(bc, dict) and bc.get("id") == item_id, str(bc)[:150])

print("\n== 6. Взять / вернуть ==")
unsigned_take = owner.call(
    "transfers.take",
    {"itemId": item_id, "dueAt": "2026-09-30T12:00:00.000Z"},
    signed=False,
)
check(
    "unsigned QR operation rejected",
    unsigned_take.get("__http") == 403
    and "DEVICE_SIGNATURE_REQUIRED" in unsigned_take.get("__body", ""),
    str(unsigned_take)[:160],
)
take = owner.call("transfers.take", {"itemId": item_id, "dueAt": "2026-09-30T12:00:00.000Z", "purpose": "монтаж"})
show("take", take)
check("take succeeds", "__err" not in take)
after = owner.call("items.byId", {"id": item_id}, mutation=False)
show("item after take", {k: after.get(k) for k in ("id", "statusName", "status", "responsibleUserId", "dueAt")} if isinstance(after, dict) else after)
take_event = next(
    (entry for entry in after.get("history", []) if entry.get("type") == "transfer_receive"),
    {},
)
check(
    "device proof embedded into V2 ledger event",
    take_event.get("eventVersion") == 2
    and take_event.get("requestDeviceId") == owner.signer.device_id
    and bool(take_event.get("requestNonce"))
    and bool(take_event.get("requestHash")),
    str(take_event)[:220],
)
ret = owner.call("transfers.returnItem", {"itemId": item_id})
show("return", ret)
check("return succeeds", "__err" not in ret)

print("\n== 7. История ==")
hist = owner.call("history.all", {"workspaceId": ws_id}, mutation=False)
n = len(hist) if isinstance(hist, list) else -1
check("history has entries", n >= 2, f"entries={n}")
if isinstance(hist, list) and hist:
    show("last history", hist[0])

print("\n== 8. Приглашение и вступление ==")
inv = owner.call("admin.workspaces.createInvite", {"workspaceId": ws_id, "role": "viewer", "maxUses": 1})
show("invite", inv)
token = inv.get("token") if isinstance(inv, dict) else None
info = Client("guest0").call("auth.inviteInfo", {"token": token}, mutation=False)
show("inviteInfo", info)
guest = Client("guest")
jr = guest.call("auth.joinRegister", {"token": token, "fullName": "Гость Наблюдатель", "phone": "+7 900 555-44-33", "password": "GuestPass12345"})
show("joinRegister", jr)
check("guest joined", isinstance(jr, dict) and "id" in jr)
jr2 = Client("guest2").call("auth.joinRegister", {"token": token, "fullName": "Второй", "phone": "+7 900 555-44-99", "password": "GuestPass12345"})
check("invite single-use enforced", "__err" in jr2, str(jr2)[:120])

check("invite carries expiry", isinstance(inv, dict) and bool(inv.get("expiresAt")), str(inv.get("expiresAt") if isinstance(inv, dict) else inv))
check("invite carries role", isinstance(inv, dict) and inv.get("role") == "viewer", str(inv.get("role") if isinstance(inv, dict) else inv))
check("inviteInfo exposes expiry", isinstance(info, dict) and bool(info.get("expiresAt")), str(info.get("expiresAt") if isinstance(info, dict) else info))

print("\n== 9. Права наблюдателя (роль viewer из приглашения) ==")
gme = guest.call("auth.me", None, mutation=False)
show("guest me", {k: gme.get(k) for k in ("id", "fullName", "position")} if isinstance(gme, dict) else gme)
check("viewer gets viewer position", isinstance(gme, dict) and gme.get("position") == "Наблюдатель", str(gme.get("position") if isinstance(gme, dict) else gme))
gcreate = guest.call("items.create", {"workspaceId": ws_id, "title": "Левый предмет", "internalId": "ВН-9999"})
check("viewer cannot create items", gcreate.get("__code") == "FORBIDDEN", str(gcreate)[:160])
gcreate2 = guest.call("items.create", {"title": "Левый предмет без ws"})
check("viewer cannot create items without workspaceId", gcreate2.get("__code") == "FORBIDDEN", str(gcreate2)[:160])
gtake = guest.call("transfers.take", {"itemId": item_id})
check("viewer cannot take items", gtake.get("__code") == "FORBIDDEN", str(gtake)[:160])

print("\n== 9b. Просроченное приглашение ==")
inv_exp = owner.call("admin.workspaces.createInvite", {"workspaceId": ws_id, "role": "member", "maxUses": 5, "expiresInHours": 1})
check("custom ttl accepted", isinstance(inv_exp, dict) and bool(inv_exp.get("expiresAt")), str(inv_exp)[:120])
lst_inv = owner.call("admin.workspaces.invites", {"workspaceId": ws_id}, mutation=False)
check("invite list reports usability", isinstance(lst_inv, list) and all("usable" in i for i in lst_inv), str(lst_inv)[:160])
used = [i for i in lst_inv if i.get("usedCount", 0) >= i.get("maxUses", 1)] if isinstance(lst_inv, list) else []
check("exhausted invite marked unusable", all(not i["usable"] for i in used), str(used)[:160])

print("\n== 10. Кросс-воркспейс доступ ==")
gitem = guest.call("items.byId", {"id": item_id}, mutation=False)
check("guest of same ws can read item", isinstance(gitem, dict) and gitem.get("id") == item_id, str(gitem)[:120])

print("\n== 11. Неисправность ==")
f = owner.call("items.reportFault", {"itemId": item_id, "severity": "high", "description": "Не держит патрон"})
show("fault", f)
check("fault reported", "__err" not in f)
st = owner.call("items.byId", {"id": item_id}, mutation=False)
show("item after fault", {k: st.get(k) for k in ("statusName", "status", "blocked")} if isinstance(st, dict) else st)
tk2 = owner.call("transfers.take", {"itemId": item_id})
check("faulty item cannot be taken", "__err" in tk2, str(tk2)[:160])

print("\n== 12. Заявка на правку ==")
cr = guest.call("items.requestChange", {"itemId": item_id, "payload": {"title": "Перфоратор Bosch GBH 2-26"}, "comment": "уточнил модель"})
show("requestChange", cr)
crs = owner.call("items.changeRequests", {"workspaceId": ws_id}, mutation=False)
show("changeRequests", crs)

print("\n== 13. Инвентаризация ==")
inv_s = owner.call("inventory.create", {"workspaceId": ws_id, "name": "Проверка августа"})
show("inventory.create", inv_s)
sid = inv_s.get("id") if isinstance(inv_s, dict) else None
chk = owner.call("inventory.checkItem", {"sessionId": sid, "itemId": item_id, "found": True, "comment": "на месте"})
show("checkItem", chk)
cmp1 = owner.call("inventory.complete", {"sessionId": sid})
cmp2 = owner.call("inventory.complete", {"sessionId": sid})
check("inventory complete idempotent/guarded", "__err" in cmp2 or cmp1 == cmp2, str(cmp2)[:120])

print("\n== 13b. Причина смены статуса ==")
stats = owner.call("admin.dictionaries.list", {"workspaceId": ws_id, "kind": "statuses"}, mutation=False)
repair = next((x for x in stats if x.get("slug") == "in-repair"), None) if isinstance(stats, list) else None
show("in-repair status", repair)
fresh = owner.call("items.create", {"workspaceId": ws_id, "title": "Шлифмашина Metabo", "storageId": st_id})
fresh_id = fresh.get("id") if isinstance(fresh, dict) else None
check("second item created", bool(fresh_id), str(fresh)[:100])
if repair and fresh_id:
    bad = owner.call("items.update", {"id": fresh_id, "statusId": repair["id"]})
    check("status change without reason rejected", bad.get("__code") == "BAD_REQUEST", str(bad)[:140])
    good = owner.call("items.update", {"id": fresh_id, "statusId": repair["id"], "reason": "Сгорел якорь"})
    check("status change with reason accepted", "__err" not in good, str(good)[:100])
    h = owner.call("history.all", {"workspaceId": ws_id}, mutation=False)
    note = (h[0].get("comment") if isinstance(h, list) and h else "") or ""
    check("ledger records status transition and reason", "Сгорел якорь" in note and "В ремонте" in note, note[:160])
else:
    check("in-repair status present", False, "no in-repair status")

print("\n== 14. Отчёты ==")
rep = owner.call("reports.allItems", {"workspaceId": ws_id}, mutation=False)
check("reports.allItems works", isinstance(rep, list), str(rep)[:120])

print("\n== 15. Чат ==")
owner.call("chat.send", {"workspaceId": ws_id, "text": "Привет, команда"})
cl = owner.call("chat.list", {"workspaceId": ws_id}, mutation=False)
check("chat works", isinstance(cl, list) and len(cl) >= 1, str(cl)[:150])

print("\n== 18. SPA-маршруты и bootstrap ==")
def status(path):
    try:
        with urllib.request.urlopen(BASE + path, timeout=10) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code


check("SPA route /tool/1 returns 200", status("/tool/1") == 200, str(status("/tool/1")))
check("invite deep link /join returns 200", status("/join?token=abc") == 200, str(status("/join?token=abc")))
check("missing asset still 404", status("/assets/nope.js") == 404, str(status("/assets/nope.js")))
opts = Client("anon2").call("auth.options", None, mutation=False)
check("registration closed after bootstrap", opts.get("registrationOpen") is False, str(opts))
check("bootstrap flag is false", opts.get("bootstrap") is False, str(opts))

print("\n== 19. Неисправность переводит в «На проверке» ==")
fresh2 = owner.call("items.create", {"workspaceId": ws_id, "title": "Дрель Makita", "storageId": st_id})
fid2 = fresh2.get("id") if isinstance(fresh2, dict) else None
owner.call("items.reportFault", {"itemId": fid2, "severity": "high", "description": "Бьёт током"})
after_fault = owner.call("items.byId", {"id": fid2}, mutation=False)
slug = (after_fault.get("status") or {}).get("slug") if isinstance(after_fault, dict) else None
check("fault sets needs-check", slug == "needs-check", str(slug))
blocked = owner.call("transfers.prepare", {"itemId": fid2, "toUserId": jr.get("id") if isinstance(jr, dict) else None})
check("blocked item cannot be transferred", "__err" in blocked, str(blocked)[:140])

print("\n== 16. CSRF / cross-site ==")
import urllib.parse
req = urllib.request.Request(f"{BASE}/api/trpc/items.create?batch=1", data=json.dumps({"0": {"json": {"workspaceId": ws_id, "name": "csrf"}}}).encode(), method="POST")
req.add_header("content-type", "application/json")
req.add_header("origin", "http://evil.example")
try:
    with owner.op.open(req, timeout=10) as r:
        print("cross-site status", r.status)
        check("cross-site mutation rejected", False)
except urllib.error.HTTPError as e:
    check("cross-site mutation rejected", e.code == 403, f"status={e.code}")

print("\n== 16b. Защитные HTTP-заголовки ==")
headers_url = f"{BASE}/api/trpc/auth.options?batch=1&input=" + urllib.parse.quote(json.dumps({"0": {"json": None}}))
with urllib.request.urlopen(headers_url, timeout=10) as response:
    headers = {k.lower(): v for k, v in response.headers.items()}
check("CSP enabled", "default-src 'self'" in headers.get("content-security-policy", ""))
check("MIME sniffing disabled", headers.get("x-content-type-options") == "nosniff")
check("API responses are not cached", headers.get("cache-control") == "no-store")
check("embedding disabled", headers.get("x-frame-options") == "DENY")

print("\n== 17. Выход ==")
lo = owner.call("auth.logout", {})
me3 = owner.call("auth.me", None, mutation=False)
check("logout revokes session", me3 in (None, {}) or "__err" in me3, str(me3)[:120])

print("\n===== ИТОГ =====")
print("failed:", len(fails))
for f_ in fails:
    print(" -", f_)

raise SystemExit(1 if fails else 0)
