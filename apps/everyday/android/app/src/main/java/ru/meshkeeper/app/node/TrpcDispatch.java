package ru.meshkeeper.app.node;

import android.database.Cursor;

import org.json.JSONArray;
import org.json.JSONObject;

final class TrpcDispatch {
    private final NodeDb n;
    private final JsonShapes j;
    private final Gossip gossip;
    private Long currentUid;

    TrpcDispatch(NodeDb n, JsonShapes j, Gossip gossip) {
        this.n = n;
        this.j = j;
        this.gossip = gossip;
    }

    Object dispatch(String proc, JSONObject in, Long uid) throws ApiEx {
        currentUid = uid;
        if (in == null) in = J.obj();
        boolean publicProc = "ping".equals(proc) || "auth.login".equals(proc)
                || "auth.register".equals(proc) || "auth.joinRegister".equals(proc)
                || "auth.inviteInfo".equals(proc) || "auth.logout".equals(proc);
        if (!publicProc) {
            n.requireUser(uid);
            guardWorkspace(proc, in, uid);
            if (proc.startsWith("admin.users.")) n.requireCan(uid, "manageUsers");
            else if (proc.startsWith("admin.workspaces.")) n.requireCan(uid, "manageWorkspaces");
            else if (proc.startsWith("admin.storages.")) n.requireCan(uid, "manageStorages");
            else if (proc.startsWith("admin.buildingSites.")) n.requireCan(uid, "manageSites");
            else if (proc.startsWith("admin.dictionaries.")) n.requireCan(uid, "manageDictionaries");
            else if (proc.startsWith("backup.")) n.requireCan(uid, "manageWorkspaces");
        }
        switch (proc) {
            case "ping": return ping();
            case "auth.directory": throw ApiEx.notFound("Процедура отключена");
            case "auth.login": return authLogin(in);
            case "auth.register": return authRegister(in);
            case "auth.join": return authJoin(in, uid);
            case "auth.joinRegister": return authJoinRegister(in);
            case "auth.logout": return okTrue();
            case "auth.me":
            case "meta.currentUser":
                n.requireUser(uid);
                return j.userPublic(uid);
            case "auth.inviteInfo": return inviteInfo(in);
            case "meta.transferCounts": return transferCounts(uid);
            case "meta.workspaces":
            case "admin.workspaces.list": return workspacesList(uid);
            case "items.list": return itemsList(in, uid);
            case "items.byId": return itemsById(in);
            case "items.byCode": return itemsByCode(in);
            case "items.nextInternalId": return itemsNextId(in, uid);
            case "items.create": return itemsCreate(in, uid);
            case "items.update": return itemsUpdate(in, uid);
            case "items.remove": return itemsRemove(in, uid);
            case "items.addPhoto": return itemsAddPhoto(in);
            case "items.addComment": return itemsAddComment(in, uid);
            case "items.reportFault": return reportFault(in, uid);
            case "items.faults": return listFaults(in);
            case "items.resolveFault": return resolveFault(in, uid);
            case "items.requestChange": return requestChange(in, uid);
            case "items.changeRequests": return listChanges(in);
            case "items.decideChange": return decideChange(in, uid);
            case "chat.list": return chatList(in, uid);
            case "chat.send": return chatSend(in, uid);
            case "sync.status": return gossip.status();
            case "sync.peers": throw ApiEx.forbidden();
            case "sync.addPeer": {
                throw ApiEx.forbidden();
            }
            case "sync.conflicts": throw ApiEx.forbidden();
            case "sync.resolveConflict": throw ApiEx.forbidden();
            case "sync.pullNow": {
                throw ApiEx.forbidden();
            }
            case "backup.export": return gossip.exportJournal();
            case "backup.import": {
                n.requireUser(uid);
                JSONObject blob = in.optJSONObject("blob");
                if (blob == null) blob = in;
                return gossip.importJournal(blob);
            }
            case "transfers.outgoing": return transfersList(uid, true);
            case "transfers.incoming": return transfersList(uid, false);
            case "transfers.byId": {
                Long id = J.lng(in, "id");
                if (id == null) throw ApiEx.bad("id");
                JSONObject t = j.transferJson(id);
                if (t == null) throw ApiEx.notFound("Передача не найдена");
                return t;
            }
            case "transfers.prepare": return transfersPrepare(in, uid);
            case "transfers.accept": return transfersAccept(in, uid, true);
            case "transfers.reject": return transfersAccept(in, uid, false);
            case "transfers.acceptAll": return transfersAcceptAll(uid);
            case "transfers.take": return transfersTake(in, uid);
            case "transfers.takeMany": return transfersTakeMany(in, uid);
            case "transfers.returnItem": return transfersReturn(in, uid);
            case "history.movements": return historyList(in, new String[]{"move", "transfer_send", "transfer_receive"});
            case "history.quantityOps":
            case "reports.quantityTransactions": return historyList(in, new String[]{"write_off", "replenish"});
            case "history.all": return historyList(in, new String[0]);
            case "history.writeOff": return historyWriteOff(in, uid);
            case "history.replenish": return historyReplenish(in, uid);
            case "history.move": return historyMove(in, uid);
            case "inventory.sessions": return invSessions(in, uid);
            case "inventory.byId": return invById(in);
            case "inventory.results": return invResults(in);
            case "inventory.create": return invCreate(in, uid);
            case "inventory.checkItem": return invCheck(in);
            case "inventory.complete": return invComplete(in, uid);
            case "notifications.list": return notifList(uid);
            case "notifications.unreadCount": return notifUnread(uid);
            case "notifications.markRead": return notifMark(in, false, uid);
            case "notifications.markAllRead": return notifMark(in, true, uid);
            case "reports.byUsers": return reportsByUsers(in, uid);
            case "reports.allItems": return reportsAll(in, uid);
            case "profile.get": return profileGet(uid);
            case "profile.update": return profileUpdate(in, uid);
            case "profile.changePassword": return profilePassword(in, uid);
            case "admin.users.list": return adminUsers(in, uid);
            case "admin.users.create": return adminUserCreate(in, uid);
            case "admin.users.update": return adminUserUpdate(in, uid);
            case "admin.users.remove": {
                Long id = J.lng(in, "id");
                n.exec("DELETE FROM users WHERE id=?", id == null ? 0 : id);
                return okTrue();
            }
            case "admin.users.invite": return adminUserInvite(in, uid);
            case "admin.users.defaultRights":
                try { return new JSONObject(NodeDb.DEFAULT_RIGHTS); } catch (Exception e) { return J.obj(); }
            case "admin.workspaces.create": return wsCreate(in);
            case "admin.workspaces.update": return wsUpdate(in);
            case "admin.workspaces.remove": {
                Long id = J.lng(in, "id");
                n.exec("DELETE FROM workspaces WHERE id=?", id == null ? 0 : id);
                return okTrue();
            }
            case "admin.workspaces.createInvite": return wsCreateInvite(in, uid);
            case "admin.workspaces.invites": return wsInvites(in, uid);
            case "admin.storages.list": return storagesList(in, uid);
            case "admin.storages.create": return storageCreate(in, uid);
            case "admin.storages.update": return storageUpdate(in);
            case "admin.storages.remove": {
                n.exec("DELETE FROM storages WHERE id=?", J.lng(in, "id"));
                return okTrue();
            }
            case "admin.buildingSites.list": return sitesList(in, uid);
            case "admin.buildingSites.create": return siteCreate(in, uid);
            case "admin.buildingSites.update": return siteUpdate(in);
            case "admin.buildingSites.remove": {
                n.exec("DELETE FROM building_sites WHERE id=?", J.lng(in, "id"));
                return okTrue();
            }
            case "admin.dictionaries.list": return dictList(in, uid);
            case "admin.dictionaries.create": return dictCreate(in, uid);
            case "admin.dictionaries.update": return dictUpdate(in);
            case "admin.dictionaries.remove": return dictRemove(in);
            default: throw ApiEx.notFound("Нет процедуры " + proc);
        }
    }

    private void guardWorkspace(String proc, JSONObject in, long uid) throws ApiEx {
        Long ws = J.lng(in, "workspaceId");
        Long itemId = J.lng(in, "itemId");
        if (itemId != null) ws = n.longN("SELECT workspace_id FROM items WHERE id=?", itemId);
        Long id = J.lng(in, "id");
        if (id != null) {
            if (proc.startsWith("items.")) ws = n.longN("SELECT workspace_id FROM items WHERE id=?", id);
            else if (proc.startsWith("transfers.")) ws = n.longN("SELECT workspace_id FROM transfers WHERE id=?", id);
            else if (proc.startsWith("inventory.")) ws = n.longN("SELECT workspace_id FROM inventory_sessions WHERE id=?", id);
            else if (proc.startsWith("admin.storages.")) ws = n.longN("SELECT workspace_id FROM storages WHERE id=?", id);
            else if (proc.startsWith("admin.buildingSites.")) ws = n.longN("SELECT workspace_id FROM building_sites WHERE id=?", id);
            else if ("admin.workspaces.update".equals(proc) || "admin.workspaces.remove".equals(proc)) ws = id;
            else if ("items.resolveFault".equals(proc)) ws = n.longN("SELECT workspace_id FROM faults WHERE id=?", id);
            else if ("items.decideChange".equals(proc)) ws = n.longN("SELECT workspace_id FROM change_requests WHERE id=?", id);
        }
        Long sid = J.lng(in, "sessionId");
        if (sid != null) ws = n.longN("SELECT workspace_id FROM inventory_sessions WHERE id=?", sid);
        if (ws != null) n.requireMember(uid, ws);
    }

    private JSONObject ping() {
        JSONObject o = J.obj();
        J.put(o, "ok", true);
        J.put(o, "ts", System.currentTimeMillis());
        J.put(o, "node", "phone");
        return o;
    }

    private JSONObject okTrue() {
        JSONObject o = J.obj();
        J.put(o, "ok", true);
        return o;
    }

    private JSONArray authDirectory() {
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, password_hash FROM users WHERE status!='disabled' ORDER BY id")) {
            while (c.moveToNext()) {
                JSONObject u = j.userPublic(c.getLong(0));
                if (u == null) continue;
                String hash = c.isNull(1) ? null : c.getString(1);
                J.put(u, "hasPassword", hash != null && !hash.isEmpty());
                a.put(u);
            }
        }
        return a;
    }

    private JSONObject authLogin(JSONObject in) throws ApiEx {
        Long id = J.lng(in, "userId");
        if (id == null) {
            String phone = J.str(in, "phone");
            if (phone == null) throw ApiEx.unauth("Укажите телефон");
            id = n.findUserPhone(phone);
            if (id == null) throw ApiEx.unauth("Аккаунт не найден. Зарегистрируйтесь или отсканируйте QR-приглашение.");
        }
        try (Cursor c = n.q("SELECT status, password_hash, full_name FROM users WHERE id=?", id)) {
            if (!c.moveToFirst()) throw ApiEx.unauth("Аккаунт не найден");
            String status = c.getString(0);
            String hash = c.isNull(1) ? null : c.getString(1);
            String name = c.getString(2);
            if ("disabled".equals(status)) throw ApiEx.unauth("Аккаунт заблокирован");
            if (hash != null && !hash.isEmpty()) {
                String pw = J.str(in, "password");
                if (pw == null || !n.verifyPassword(pw, hash)) throw ApiEx.unauth("Неверный пароль");
            } else throw ApiEx.unauth("Для аккаунта ещё не установлен пароль");
            if ("invited".equals(status)) n.exec("UPDATE users SET status='active' WHERE id=?", id);
            JSONObject u = j.userPublic(id);
            if (u == null) throw ApiEx.unauth("Аккаунт не найден");
            J.put(u, "fullName", name);
            return u;
        }
    }

    private JSONObject authRegister(JSONObject in) throws ApiEx {
        String fullName = J.str(in, "fullName");
        String phone = J.str(in, "phone");
        String password = J.str(in, "password");
        if (fullName == null) throw ApiEx.bad("Введите имя");
        if (phone == null) throw ApiEx.bad("Введите телефон");
        if (password == null) throw ApiEx.bad("Введите пароль");
        if (password.length() < 10) throw ApiEx.bad("Пароль минимум 10 символов");
        if (n.long1("SELECT COUNT(*) FROM users") > 0) throw ApiEx.forbidden();
        if (n.findUserPhone(phone) != null) {
            throw ApiEx.conflict("Этот телефон уже зарегистрирован. Войдите с тем же номером и паролем.");
        }
        String wsName = J.str(in, "workspaceName");
        if (wsName == null) wsName = "Моя группа";
        String syncUrl = J.str(in, "syncUrl");
        long ws = n.insert("INSERT INTO workspaces (name, timezone, internal_id_prefix, comment, created_at, sync_url, guid) VALUES (?,?,?,?,?,?,?)",
                wsName, "Europe/Moscow", "ВН-", "Создано на телефоне", NodeDb.now(), syncUrl, NodeDb.guid());
        n.seedWorkspaceStatuses(ws);
        if (syncUrl != null) gossip.addPeer(syncUrl, "relay", null);
        long uid = n.insert("INSERT INTO users (full_name, position, phone, status, password_hash, role_rights, pubkey, privkey, created_at, guid, checkout_policy) VALUES (?,'Владелец',?,'active',?,?,?,?,?,?,?)",
                fullName, phone, n.hashPassword(password), NodeDb.OWNER_RIGHTS, NodeDb.guid(), NodeDb.guid(), NodeDb.now(), NodeDb.guid(), NodeDb.DEFAULT_POLICY);
        n.exec("INSERT INTO user_workspaces (user_id, workspace_id) VALUES (?,?)", uid, ws);
        n.exec("INSERT INTO storages (name, responsible_user_id, workspace_id, address) VALUES ('Основной склад',?,?,'')", uid, ws);
        JSONObject u = j.userPublic(uid);
        if (u == null) throw ApiEx.bad("не создан");
        return u;
    }

    private long[] inviteByToken(String token) throws ApiEx {
        try (Cursor c = n.q("SELECT id, workspace_id, role, max_uses, used_count, revoked FROM invites WHERE token=?", token)) {
            if (!c.moveToFirst()) throw ApiEx.notFound("Приглашение недействительно или истекло");
            return new long[]{c.getLong(0), c.getLong(1), c.getLong(3), c.getLong(4), c.getLong(5)};
        }
    }

    private JSONObject consumeInvite(String token, long userId) throws ApiEx {
        long[] inv = inviteByToken(token);
        long id = inv[0], ws = inv[1], maxUses = inv[2], used = inv[3], revoked = inv[4];
        if (revoked != 0) throw ApiEx.bad("Приглашение отозвано");
        if (used >= maxUses) throw ApiEx.bad("Приглашение уже использовано");
        long exists = n.long1("SELECT COUNT(*) FROM user_workspaces WHERE user_id=? AND workspace_id=?", userId, ws);
        if (exists == 0) n.exec("INSERT INTO user_workspaces (user_id, workspace_id) VALUES (?,?)", userId, ws);
        n.exec("UPDATE invites SET used_count=used_count+1 WHERE id=?", id);
        JSONObject wsj = j.workspaceJson(ws);
        return wsj == null ? J.obj() : wsj;
    }

    private JSONObject authJoin(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        String token = J.str(in, "token");
        if (token == null) throw ApiEx.bad("Нет токена приглашения");
        return consumeInvite(token, uid);
    }

    private JSONObject authJoinRegister(JSONObject in) throws ApiEx {
        String token = J.str(in, "token");
        if (token == null) throw ApiEx.bad("Нет токена приглашения");
        long[] inv = inviteByToken(token);
        long ws = inv[1];
        String fullName = J.str(in, "fullName");
        String phone = J.str(in, "phone");
        String password = J.str(in, "password");
        if (fullName == null) throw ApiEx.bad("Введите имя");
        if (phone == null) throw ApiEx.bad("Введите телефон");
        Long uid = n.findUserPhone(phone);
        if (uid != null) {
            String h = n.str1("SELECT password_hash FROM users WHERE id=?", uid);
            if (h != null && !h.isEmpty()) {
                if (password == null || password.length() < 6 || !n.verifyPassword(password, h)) {
                    throw ApiEx.unauth("Неверный пароль для этого телефона");
                }
            } else {
                long invitedHere = n.long1("SELECT COUNT(*) FROM user_workspaces uw JOIN users u ON u.id=uw.user_id WHERE uw.user_id=? AND uw.workspace_id=? AND u.status='invited'", uid, ws);
                if (invitedHere == 0 || password == null || password.length() < 10) throw ApiEx.unauth("Аккаунт требует персональной активации");
                n.exec("UPDATE users SET password_hash=?,status='active' WHERE id=?", n.hashPassword(password), uid);
            }
        } else {
            if (password == null || password.length() < 10) throw ApiEx.bad("Пароль минимум 10 символов");
            uid = n.insert("INSERT INTO users (full_name, position, phone, status, password_hash, role_rights, pubkey, privkey, created_at, guid, checkout_policy) VALUES (?,'Участник',?,'active',?,?,?,?,?,?,?)",
                    fullName, phone, n.hashPassword(password), NodeDb.DEFAULT_RIGHTS, NodeDb.guid(), NodeDb.guid(), NodeDb.now(), NodeDb.guid(), NodeDb.DEFAULT_POLICY);
        }
        JSONObject wsj = consumeInvite(token, uid);
        JSONObject u = j.userPublic(uid);
        if (u == null) throw ApiEx.bad("ошибка");
        J.put(u, "joinedWorkspace", wsj);
        J.put(u, "workspaceId", ws);
        return u;
    }

    private JSONObject inviteInfo(JSONObject in) throws ApiEx {
        String token = J.str(in, "token");
        if (token == null) throw ApiEx.bad("token");
        long[] inv = inviteByToken(token);
        if (inv[4] != 0 || inv[3] >= inv[2]) throw ApiEx.bad("Приглашение больше не действует");
        JSONObject out = J.obj();
        J.put(out, "workspace", j.workspaceJson(inv[1]));
        J.put(out, "role", "member");
        J.put(out, "token", token);
        return out;
    }

    private JSONArray workspacesList(Long uid) {
        JSONArray a = J.arr();
        String sql = uid != null
                ? "SELECT workspace_id FROM user_workspaces WHERE user_id=? ORDER BY id"
                : "SELECT id FROM workspaces ORDER BY id";
        try (Cursor c = uid != null ? n.q(sql, uid) : n.q(sql)) {
            while (c.moveToNext()) {
                JSONObject w = j.workspaceJson(c.getLong(0));
                if (w != null) a.put(w);
            }
        }
        return a;
    }

    private JSONObject transferCounts(Long uid) throws ApiEx {
        n.requireUser(uid);
        JSONObject o = J.obj();
        J.put(o, "outgoing", n.long1("SELECT COUNT(*) FROM transfers WHERE from_user_id=? AND status IN ('draft','pending')", uid));
        J.put(o, "incoming", n.long1("SELECT COUNT(*) FROM transfers WHERE to_user_id=? AND status='pending'", uid));
        return o;
    }

    private JSONObject itemsList(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        long page = J.lng(in, "page") == null ? 1 : Math.max(1, J.lng(in, "page"));
        long limit = J.lng(in, "limit") == null ? 20 : Math.max(1, Math.min(500, J.lng(in, "limit")));
        String search = J.str(in, "search");
        boolean onlyMine = Boolean.TRUE.equals(J.bool(in, "onlyMine"));
        JSONArray ids = J.arr();
        try (Cursor c = n.q("SELECT id, title, internal_id, serial_number, responsible_user_id FROM items WHERE workspace_id=? ORDER BY created_at DESC, id DESC", ws)) {
            while (c.moveToNext()) {
                long id = c.getLong(0);
                if (onlyMine) {
                    if (c.isNull(4) || uid == null || c.getLong(4) != uid) continue;
                }
                if (search != null) {
                    String blob = (c.getString(1) + " " + c.getString(2) + " " + (c.isNull(3) ? "" : c.getString(3))).toLowerCase();
                    if (!blob.contains(search.toLowerCase())) continue;
                }
                ids.put(id);
            }
        }
        int total = ids.length();
        int start = (int) ((page - 1) * limit);
        JSONArray rows = J.arr();
        for (int i = start; i < total && i < start + limit; i++) {
            JSONObject it = j.itemJson(ids.optLong(i), false);
            if (it != null) rows.put(it);
        }
        JSONObject out = J.obj();
        J.put(out, "rows", rows);
        J.put(out, "page", page);
        J.put(out, "limit", limit);
        J.put(out, "hasMore", total > start + limit);
        J.put(out, "total", total);
        return out;
    }

    private JSONObject itemsById(JSONObject in) throws ApiEx {
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        JSONObject it = j.itemJson(id, true);
        if (it == null) throw ApiEx.notFound("Инструмент не найден");
        return it;
    }

    private JSONObject itemsByCode(JSONObject in) throws ApiEx {
        String code = J.str(in, "code");
        if (code == null) throw ApiEx.bad("code");
        Long id = n.longN("SELECT id FROM items WHERE qr_code=? OR internal_id=? LIMIT 1", code, code);
        if (id == null) throw ApiEx.notFound("Инструмент с таким QR/номером не найден");
        JSONObject it = j.itemJson(id, false);
        if (it == null) throw ApiEx.notFound("Инструмент не найден");
        return it;
    }

    private String itemsNextId(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        String prefix = n.str1("SELECT internal_id_prefix FROM workspaces WHERE id=?", ws);
        if (prefix == null) prefix = "ВН-";
        long max = 0;
        try (Cursor c = n.q("SELECT internal_id FROM items WHERE workspace_id=?", ws)) {
            while (c.moveToNext()) {
                String id = c.getString(0);
                if (id != null && id.startsWith(prefix)) {
                    try {
                        long nmb = Long.parseLong(id.substring(prefix.length()));
                        if (nmb > max) max = nmb;
                    } catch (Exception ignored) {}
                }
            }
        }
        return prefix + String.format(java.util.Locale.US, "%04d", max + 1);
    }

    private JSONObject itemsCreate(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        n.requireCan(uid, "createItems");
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        String title = J.str(in, "title");
        if (title == null) throw ApiEx.bad("Название обязательно");
        String internal = J.str(in, "internalId");
        if (internal == null) internal = itemsNextId(in, uid);
        String qr = J.str(in, "qrCode");
        if (qr == null) qr = internal;
        Long stockId = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug='in-stock'", ws);
        Long statusId = J.lng(in, "statusId");
        if (statusId == null) statusId = stockId;
        long id = n.insert("INSERT INTO items (internal_id,title,category_id,brand_id,status_id,responsible_user_id,building_site_id,storage_id,workspace_id,serial_number,cost,quantitative,quantity,unit,comment,qr_code,created_at,guid) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                internal, title, J.lng(in, "categoryId"), J.lng(in, "brandId"), statusId,
                J.lng(in, "responsibleUserId"), J.lng(in, "buildingSiteId"), J.lng(in, "storageId"), ws,
                J.str(in, "serialNumber"), J.dbl(in, "cost"), Boolean.TRUE.equals(J.bool(in, "quantitative")) ? 1 : 0,
                J.dbl(in, "quantity"), J.str(in, "unit"), J.str(in, "comment"), qr, NodeDb.now(), NodeDb.guid());
        JSONArray photos = J.arr(in, "photos");
        if (photos != null) {
            for (int i = 0; i < photos.length(); i++) {
                String url = photos.optString(i, null);
                if (url != null && !url.isEmpty()) n.exec("INSERT INTO item_photos (item_id,url,is_title) VALUES (?,?,?)", id, url, i == 0 ? 1 : 0);
            }
        }
        n.appendLedger(ws, uid, id, "create", null, title, null, "Инструмент добавлен в каталог");
        return j.itemJson(id, true);
    }

    private JSONObject itemsUpdate(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        n.requireCan(uid, "editItems");
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        JSONObject before = j.itemJson(id, false);
        if (before == null) throw ApiEx.notFound("Инструмент не найден");
        n.exec("UPDATE items SET title=COALESCE(?,title), category_id=COALESCE(?,category_id), brand_id=COALESCE(?,brand_id), status_id=COALESCE(?,status_id), responsible_user_id=?, building_site_id=?, storage_id=?, serial_number=COALESCE(?,serial_number), cost=COALESCE(?,cost), comment=COALESCE(?,comment), qr_code=COALESCE(?,qr_code), calibrated_until=COALESCE(?,calibrated_until), min_quantity=COALESCE(?,min_quantity) WHERE id=?",
                J.str(in, "title"), J.lng(in, "categoryId"), J.lng(in, "brandId"), J.lng(in, "statusId"),
                J.lng(in, "responsibleUserId"), J.lng(in, "buildingSiteId"), J.lng(in, "storageId"),
                J.str(in, "serialNumber"), J.dbl(in, "cost"), J.str(in, "comment"), J.str(in, "qrCode"),
                J.str(in, "calibratedUntil"), J.dbl(in, "minQuantity"), id);
        n.appendLedger(before.optLong("workspaceId", 1), uid, id, "update", null, null, null, "Данные инструмента обновлены");
        return j.itemJson(id, true);
    }

    private JSONObject itemsRemove(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        n.requireCan(uid, "deleteItems");
        Long id = J.lng(in, "id");
        n.exec("DELETE FROM item_photos WHERE item_id=?", id);
        n.exec("DELETE FROM items WHERE id=?", id);
        return okTrue();
    }

    private JSONObject itemsAddPhoto(JSONObject in) throws ApiEx {
        Long itemId = J.lng(in, "itemId");
        String url = J.str(in, "url");
        if (itemId == null) throw ApiEx.bad("itemId");
        if (url == null) throw ApiEx.bad("url");
        boolean title = Boolean.TRUE.equals(J.bool(in, "isTitle"));
        long id = n.insert("INSERT INTO item_photos (item_id,url,is_title) VALUES (?,?,?)", itemId, url, title ? 1 : 0);
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "itemId", itemId);
        J.put(o, "url", url);
        J.put(o, "isTitle", title);
        return o;
    }

    private JSONObject itemsAddComment(JSONObject in, Long uid) throws ApiEx {
        Long user = J.lng(in, "userId");
        if (user == null) user = uid;
        if (user == null) throw ApiEx.unauth("нет пользователя");
        Long itemId = J.lng(in, "itemId");
        String text = J.str(in, "text");
        if (itemId == null) throw ApiEx.bad("itemId");
        if (text == null) throw ApiEx.bad("text");
        long id = n.insert("INSERT INTO item_comments (item_id,user_id,text,created_at) VALUES (?,?,?,?)", itemId, user, text, NodeDb.now());
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "itemId", itemId);
        J.put(o, "userId", user);
        J.put(o, "text", text);
        J.put(o, "user", j.userPublic(user));
        return o;
    }

    private JSONObject takeOne(long uid, long itemId, String comment, String dueAt, String photoUrl, Double qty) throws ApiEx {
        n.requireCan(uid, "transferItems");
        JSONObject item = j.itemJson(itemId, false);
        if (item == null) throw ApiEx.notFound("Инструмент не найден");
        JSONObject st = item.optJSONObject("status");
        String slug = st == null ? "" : st.optString("slug", "");
        if ("written-off".equals(slug)) throw ApiEx.bad("Списанный инструмент нельзя взять");
        if ("in-repair".equals(slug) || "needs-check".equals(slug)) throw ApiEx.bad("Инструмент на проверке или в ремонте, выдача запрещена");
        boolean quantitative = item.optBoolean("quantitative", false);
        if (!quantitative && item.optLong("responsibleUserId", -1) == uid && !item.isNull("responsibleUserId")) {
            throw ApiEx.bad("Инструмент уже у вас");
        }
        long ws = item.optLong("workspaceId", 1);
        Long from = item.isNull("responsibleUserId") ? null : item.optLong("responsibleUserId");
        if (from == null) {
            JSONObject storage = item.optJSONObject("storage");
            if (storage != null && !storage.isNull("responsibleUserId")) from = storage.optLong("responsibleUserId");
        }
        if (from == null) from = uid;
        String code = nextTransferCode(ws);
        if (quantitative) {
            double takeQty = qty == null ? 1 : qty;
            if (takeQty <= 0) throw ApiEx.bad("Укажите количество");
            double stock = item.optDouble("quantity", 0);
            if (takeQty > stock + 1e-9) throw ApiEx.bad("На складе только " + stock);
            n.exec("UPDATE items SET quantity=? WHERE id=?", stock - takeQty, itemId);
            n.exec("INSERT INTO item_holdings (item_id,user_id,quantity,due_at,comment,photo_url,created_at) VALUES (?,?,?,?,?,?,?)",
                    itemId, uid, takeQty, dueAt, comment, photoUrl, NodeDb.now());
            n.exec("INSERT INTO transfers (code,item_id,from_user_id,to_user_id,to_storage_id,building_site_id,workspace_id,quantity,status,comment,no_confirmation,photo_url,created_at,completed_at) VALUES (?,?,?,?,?,?,?,?,'accepted',?,1,?,?,?)",
                    code, itemId, from, uid, item.isNull("storageId") ? null : item.optLong("storageId"),
                    item.isNull("buildingSiteId") ? null : item.optLong("buildingSiteId"), ws, takeQty, comment, photoUrl, NodeDb.now(), NodeDb.now());
            String toName = nameOf(uid);
            n.appendLedger(ws, uid, itemId, "transfer_receive", "Склад", toName, takeQty, "Выдача " + code);
            return j.itemJson(itemId, false);
        }
        Long inWork = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug='in-work'", ws);
        n.exec("INSERT INTO transfers (code,item_id,from_user_id,to_user_id,to_storage_id,building_site_id,workspace_id,status,comment,no_confirmation,photo_url,created_at,completed_at) VALUES (?,?,?,?,?,?,?,'accepted',?,1,?,?,?)",
                code, itemId, from, uid, item.isNull("storageId") ? null : item.optLong("storageId"),
                item.isNull("buildingSiteId") ? null : item.optLong("buildingSiteId"), ws, comment, photoUrl, NodeDb.now(), NodeDb.now());
        n.exec("UPDATE items SET responsible_user_id=?, status_id=COALESCE(?,status_id), due_at=? WHERE id=?", uid, inWork, dueAt, itemId);
        String fromName = nameOf(from);
        String toName = nameOf(uid);
        n.appendLedger(ws, from, itemId, "transfer_send", fromName, toName, null, "Выдача " + code);
        n.appendLedger(ws, uid, itemId, "transfer_receive", fromName, toName, null, "Получение " + code);
        return j.itemJson(itemId, false);
    }

    private JSONObject transfersTake(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long id = J.lng(in, "itemId");
        if (id == null) throw ApiEx.bad("itemId");
        return takeOne(uid, id, J.str(in, "comment"), J.str(in, "dueAt"), J.str(in, "photoUrl"), J.dbl(in, "quantity"));
    }

    private JSONObject transfersTakeMany(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        JSONArray ids = J.arr(in, "itemIds");
        JSONArray taken = J.arr();
        JSONArray failed = J.arr();
        if (ids != null) {
            for (int i = 0; i < ids.length(); i++) {
                long id = ids.optLong(i);
                try {
                    takeOne(uid, id, J.str(in, "comment"), J.str(in, "dueAt"), J.str(in, "photoUrl"), J.dbl(in, "quantity"));
                    taken.put(id);
                } catch (ApiEx e) {
                    JSONObject f = J.obj();
                    J.put(f, "itemId", id);
                    J.put(f, "message", e.getMessage());
                    failed.put(f);
                }
            }
        }
        JSONObject o = J.obj();
        J.put(o, "takenCount", taken.length());
        J.put(o, "taken", taken);
        J.put(o, "failed", failed);
        return o;
    }

    private JSONObject transfersReturn(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long id = J.lng(in, "itemId");
        if (id == null) throw ApiEx.bad("itemId");
        JSONObject item = j.itemJson(id, false);
        if (item == null) throw ApiEx.notFound("Инструмент не найден");
        if (item.optBoolean("quantitative", false)) {
            double held = 0;
            try (Cursor c = n.q("SELECT COALESCE(SUM(quantity),0) FROM item_holdings WHERE item_id=? AND user_id=? AND returned_at IS NULL", id, uid)) {
                if (c.moveToFirst()) held = c.getDouble(0);
            }
            if (held <= 0) throw ApiEx.bad("У вас нет этого материала");
            Double q = J.dbl(in, "quantity");
            double give = q == null ? held : Math.min(q, held);
            n.exec("UPDATE item_holdings SET returned_at=? WHERE item_id=? AND user_id=? AND returned_at IS NULL", NodeDb.now(), id, uid);
            if (give + 1e-9 < held) {
                n.exec("INSERT INTO item_holdings (item_id,user_id,quantity,created_at) VALUES (?,?,?,?)", id, uid, held - give, NodeDb.now());
            }
            double stock = item.optDouble("quantity", 0) + give;
            n.exec("UPDATE items SET quantity=? WHERE id=?", stock, id);
            n.appendLedger(item.optLong("workspaceId", 1), uid, id, "transfer_send", nameOf(uid), "Склад", give, "Возврат");
            return j.itemJson(id, false);
        }
        if (item.isNull("responsibleUserId")) throw ApiEx.bad("Инструмент уже на складе");
        if (item.optLong("responsibleUserId") != uid) throw ApiEx.bad("Инструмент на другом сотруднике");
        long ws = item.optLong("workspaceId", 1);
        Long inStock = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug='in-stock'", ws);
        n.exec("UPDATE items SET responsible_user_id=NULL, building_site_id=NULL, status_id=COALESCE(?,status_id), due_at=NULL WHERE id=?", inStock, id);
        n.appendLedger(ws, uid, id, "transfer_send", nameOf(uid), "Склад", null, "Возврат на склад");
        return j.itemJson(id, false);
    }

    private JSONArray transfersList(Long uid, boolean outgoing) throws ApiEx {
        n.requireUser(uid);
        String sql = outgoing
                ? "SELECT id FROM transfers WHERE from_user_id=? AND status IN ('draft','pending') ORDER BY id DESC"
                : "SELECT id FROM transfers WHERE to_user_id=? AND status='pending' ORDER BY id DESC";
        JSONArray a = J.arr();
        try (Cursor c = n.q(sql, uid)) {
            while (c.moveToNext()) {
                JSONObject t = j.transferJson(c.getLong(0));
                if (t != null) a.put(t);
            }
        }
        return a;
    }

    private JSONObject transfersPrepare(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long itemId = J.lng(in, "itemId");
        Long to = J.lng(in, "toUserId");
        if (itemId == null) throw ApiEx.bad("itemId");
        if (to == null) throw ApiEx.bad("toUserId");
        JSONObject item = j.itemJson(itemId, false);
        if (item == null) throw ApiEx.notFound("Инструмент не найден");
        long ws = item.optLong("workspaceId", 1);
        String status = Boolean.TRUE.equals(J.bool(in, "asDraft")) ? "draft" : "pending";
        String code = nextTransferCode(ws);
        long tid = n.insert("INSERT INTO transfers (code,item_id,from_user_id,to_user_id,to_storage_id,building_site_id,workspace_id,quantity,status,comment,no_confirmation,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                code, itemId, uid, to, J.lng(in, "toStorageId"), J.lng(in, "buildingSiteId"), ws, J.dbl(in, "quantity"), status, J.str(in, "comment"), Boolean.TRUE.equals(J.bool(in, "noConfirmation")) ? 1 : 0, NodeDb.now());
        n.appendLedger(ws, uid, itemId, "transfer_send", null, null, null, "Передача " + code);
        if (!to.equals(uid)) {
            n.exec("INSERT INTO notifications (user_id,item_id,type,title,text,created_at) VALUES (?,?,'transfer','Ожидает приёма',?,?)",
                    to, itemId, "Передача " + code + ": " + item.optString("title"), NodeDb.now());
        }
        return j.transferJson(tid);
    }

    private JSONObject transfersAccept(JSONObject in, Long uid, boolean accept) throws ApiEx {
        n.requireUser(uid);
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        JSONObject t = j.transferJson(id);
        if (t == null) throw ApiEx.notFound("Передача не найдена");
        String st = t.optString("status", "");
        if (!"pending".equals(st) && !"draft".equals(st)) throw ApiEx.bad("Передача уже завершена");
        String newSt = accept ? "accepted" : "rejected";
        n.exec("UPDATE transfers SET status=?, completed_at=? WHERE id=?", newSt, NodeDb.now(), id);
        if (accept) {
            long itemId = t.optLong("itemId");
            long to = t.optLong("toUserId");
            long ws = t.optLong("workspaceId", 1);
            Long inWork = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug='in-work'", ws);
            n.exec("UPDATE items SET responsible_user_id=?, status_id=COALESCE(?,status_id) WHERE id=?", to, inWork, itemId);
            n.appendLedger(ws, uid, itemId, "transfer_receive", null, nameOf(to), null, "Приём передачи");
        }
        return j.transferJson(id);
    }

    private JSONObject transfersAcceptAll(Long uid) throws ApiEx {
        n.requireUser(uid);
        JSONArray incoming = transfersList(uid, false);
        int nOk = 0;
        for (int i = 0; i < incoming.length(); i++) {
            JSONObject t = incoming.optJSONObject(i);
            if (t == null) continue;
            JSONObject in = J.obj();
            J.put(in, "id", t.optLong("id"));
            try { transfersAccept(in, uid, true); nOk++; } catch (ApiEx ignored) {}
        }
        JSONObject o = J.obj();
        J.put(o, "ok", true);
        J.put(o, "accepted", nOk);
        return o;
    }

    private JSONArray historyList(JSONObject in, String[] types) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(currentUid);
        JSONArray a = J.arr();
        String sql = "SELECT id, type FROM history_entries WHERE workspace_id=? ORDER BY id DESC LIMIT 200";
        try (Cursor c = n.q(sql, ws)) {
            while (c.moveToNext()) {
                if (types.length > 0) {
                    String t = c.getString(1);
                    boolean ok = false;
                    for (String x : types) if (x.equals(t)) { ok = true; break; }
                    if (!ok) continue;
                }
                JSONObject h = j.historyJson(c.getLong(0));
                if (h != null) a.put(h);
            }
        }
        return a;
    }

    private JSONObject historyWriteOff(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        n.requireCan(uid, "writeOff");
        if (J.str(in, "comment") == null) throw ApiEx.bad("Укажите причину списания");
        Long id = J.lng(in, "itemId");
        if (id == null) throw ApiEx.bad("itemId");
        JSONObject item = j.itemJson(id, false);
        if (item == null) throw ApiEx.notFound("нет");
        long ws = item.optLong("workspaceId", 1);
        if (item.optBoolean("quantitative", false)) {
            double qty = J.dbl(in, "quantity") == null ? 1 : J.dbl(in, "quantity");
            n.exec("UPDATE items SET quantity=? WHERE id=?", Math.max(0, item.optDouble("quantity", 0) - qty), id);
            n.appendLedger(ws, uid, id, "write_off", null, null, -qty, J.str(in, "comment"));
        } else {
            Long st = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug='written-off'", ws);
            if (st != null) n.exec("UPDATE items SET status_id=? WHERE id=?", st, id);
            n.appendLedger(ws, uid, id, "write_off", null, null, null, J.str(in, "comment"));
        }
        return j.itemJson(id, false);
    }

    private JSONObject historyReplenish(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        n.requireCan(uid, "replenish");
        Long id = J.lng(in, "itemId");
        Double qty = J.dbl(in, "quantity");
        if (id == null) throw ApiEx.bad("itemId");
        if (qty == null) throw ApiEx.bad("quantity");
        JSONObject item = j.itemJson(id, false);
        if (item == null) throw ApiEx.notFound("нет");
        if (!item.optBoolean("quantitative", false)) throw ApiEx.bad("Инструмент не количественный");
        n.exec("UPDATE items SET quantity=? WHERE id=?", item.optDouble("quantity", 0) + qty, id);
        n.appendLedger(item.optLong("workspaceId", 1), uid, id, "replenish", null, null, qty, J.str(in, "comment"));
        return j.itemJson(id, false);
    }

    private JSONObject historyMove(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long id = J.lng(in, "itemId");
        if (id == null) throw ApiEx.bad("itemId");
        n.exec("UPDATE items SET storage_id=COALESCE(?,storage_id), building_site_id=? WHERE id=?", J.lng(in, "toStorageId"), J.lng(in, "toBuildingSiteId"), id);
        JSONObject item = j.itemJson(id, false);
        if (item == null) throw ApiEx.notFound("нет");
        n.appendLedger(item.optLong("workspaceId", 1), uid, id, "move", null, null, null, J.str(in, "comment"));
        return item;
    }

    private JSONArray invSessions(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, number, workspace_id, status, started_by, created_at, completed_at FROM inventory_sessions WHERE workspace_id=? ORDER BY id DESC", ws)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                long id = c.getLong(0);
                J.put(o, "id", id);
                J.put(o, "number", c.getString(1));
                J.put(o, "workspaceId", c.getLong(2));
                J.put(o, "status", c.getString(3));
                J.put(o, "startedBy", c.getLong(4));
                J.put(o, "createdAt", c.getString(5));
                J.put(o, "completedAt", c.isNull(6) ? null : c.getString(6));
                J.put(o, "total", n.long1("SELECT COUNT(*) FROM inventory_results WHERE session_id=?", id));
                J.put(o, "checked", n.long1("SELECT COUNT(*) FROM inventory_results WHERE session_id=? AND checked=1", id));
                a.put(o);
            }
        }
        return a;
    }

    private JSONObject invById(JSONObject in) throws ApiEx {
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        try (Cursor c = n.q("SELECT id, number, workspace_id, status, started_by, created_at, completed_at FROM inventory_sessions WHERE id=?", id)) {
            if (!c.moveToFirst()) throw ApiEx.notFound("нет");
            JSONObject o = J.obj();
            J.put(o, "id", c.getLong(0));
            J.put(o, "number", c.getString(1));
            J.put(o, "workspaceId", c.getLong(2));
            J.put(o, "status", c.getString(3));
            J.put(o, "startedBy", c.getLong(4));
            J.put(o, "createdAt", c.getString(5));
            J.put(o, "completedAt", c.isNull(6) ? null : c.getString(6));
            J.put(o, "results", invResults(in));
            return o;
        }
    }

    private JSONArray invResults(JSONObject in) {
        Long sid = J.lng(in, "sessionId");
        if (sid == null) sid = J.lng(in, "id");
        JSONArray a = J.arr();
        if (sid == null) return a;
        try (Cursor c = n.q("SELECT id, session_id, item_id, expected_qty, actual_qty, checked FROM inventory_results WHERE session_id=?", sid)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "sessionId", c.getLong(1));
                J.put(o, "itemId", c.getLong(2));
                J.put(o, "expectedQty", c.isNull(3) ? null : c.getDouble(3));
                J.put(o, "actualQty", c.isNull(4) ? null : c.getDouble(4));
                J.put(o, "checked", c.getLong(5) != 0);
                J.put(o, "item", j.itemJson(c.getLong(2), false));
                a.put(o);
            }
        }
        return a;
    }

    private JSONObject invCreate(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        long nSess = n.long1("SELECT COUNT(*) FROM inventory_sessions WHERE workspace_id=?", ws);
        String number = "ИН-" + String.format(java.util.Locale.US, "%04d", nSess + 1);
        long id = n.insert("INSERT INTO inventory_sessions (number,workspace_id,started_by,created_at) VALUES (?,?,?,?)", number, ws, uid, NodeDb.now());
        try (Cursor c = n.q("SELECT id, quantity FROM items WHERE workspace_id=?", ws)) {
            while (c.moveToNext()) {
                n.exec("INSERT INTO inventory_results (session_id,item_id,expected_qty,checked) VALUES (?,?,?,0)", id, c.getLong(0), c.isNull(1) ? 1 : c.getDouble(1));
            }
        }
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "number", number);
        J.put(o, "workspaceId", ws);
        return o;
    }

    private JSONObject invCheck(JSONObject in) throws ApiEx {
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        n.exec("UPDATE inventory_results SET actual_qty=?, checked=1 WHERE id=?", J.dbl(in, "actualQty"), id);
        return okTrue();
    }

    private JSONObject invComplete(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long id = J.lng(in, "id");
        n.exec("UPDATE inventory_sessions SET status='completed', completed_at=? WHERE id=?", NodeDb.now(), id);
        return okTrue();
    }

    private JSONArray notifList(Long uid) throws ApiEx {
        n.requireUser(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, user_id, item_id, type, title, text, read, created_at FROM notifications WHERE user_id=? ORDER BY id DESC LIMIT 100", uid)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                Long itemId = c.isNull(2) ? null : c.getLong(2);
                J.put(o, "id", c.getLong(0));
                J.put(o, "userId", c.getLong(1));
                J.put(o, "itemId", itemId);
                J.put(o, "type", c.getString(3));
                J.put(o, "title", c.isNull(4) ? null : c.getString(4));
                J.put(o, "text", c.getString(5));
                J.put(o, "read", c.getLong(6) != 0);
                J.put(o, "createdAt", c.getString(7));
                J.put(o, "item", itemId == null ? null : j.itemJson(itemId, false));
                a.put(o);
            }
        }
        return a;
    }

    private JSONObject notifUnread(Long uid) throws ApiEx {
        n.requireUser(uid);
        JSONObject o = J.obj();
        J.put(o, "count", n.long1("SELECT COUNT(*) FROM notifications WHERE user_id=? AND read=0", uid));
        return o;
    }

    private JSONObject notifMark(JSONObject in, boolean all, Long uid) throws ApiEx {
        n.requireUser(uid);
        if (all) n.exec("UPDATE notifications SET read=1 WHERE user_id=? AND read=0", uid);
        else if (J.lng(in, "id") != null) n.exec("UPDATE notifications SET read=1 WHERE id=?", J.lng(in, "id"));
        return okTrue();
    }

    private JSONArray reportsByUsers(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT DISTINCT responsible_user_id FROM items WHERE workspace_id=?", ws)) {
            while (c.moveToNext()) {
                Long u = c.isNull(0) ? null : c.getLong(0);
                JSONArray items = J.arr();
                double total = 0;
                String sql = u == null
                        ? "SELECT id FROM items WHERE workspace_id=? AND responsible_user_id IS NULL"
                        : "SELECT id FROM items WHERE workspace_id=? AND responsible_user_id=?";
                try (Cursor ic = u == null ? n.q(sql, ws) : n.q(sql, ws, u)) {
                    while (ic.moveToNext()) {
                        JSONObject it = j.itemJson(ic.getLong(0), false);
                        if (it != null) {
                            items.put(it);
                            total += it.optDouble("cost", 0);
                        }
                    }
                }
                JSONObject o = J.obj();
                J.put(o, "userId", u);
                J.put(o, "user", userSafe(u));
                J.put(o, "itemsCount", items.length());
                J.put(o, "totalCost", total);
                J.put(o, "items", items);
                a.put(o);
            }
        }
        return a;
    }

    private JSONArray reportsAll(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id FROM items WHERE workspace_id=? ORDER BY created_at DESC", ws)) {
            while (c.moveToNext()) {
                JSONObject it = j.itemJson(c.getLong(0), false);
                if (it != null) a.put(it);
            }
        }
        return a;
    }

    private JSONObject profileGet(Long uid) throws ApiEx {
        n.requireUser(uid);
        JSONObject u = j.userPublic(uid);
        if (u == null) throw ApiEx.unauth("нет");
        J.put(u, "workspaces", workspacesList(uid));
        return u;
    }

    private JSONObject profileUpdate(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        n.exec("UPDATE users SET full_name=COALESCE(?,full_name), position=COALESCE(?,position), phone=COALESCE(?,phone), avatar_url=COALESCE(?,avatar_url) WHERE id=?",
                J.str(in, "fullName"), J.str(in, "position"), J.str(in, "phone"), J.str(in, "avatarUrl"), uid);
        return j.userPublic(uid);
    }

    private JSONObject profilePassword(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        String newp = J.str(in, "newPassword");
        if (newp == null || newp.length() < 6) throw ApiEx.bad("Пароль минимум 6 символов");
        String old = n.str1("SELECT password_hash FROM users WHERE id=?", uid);
        if (old != null && !old.isEmpty()) {
            String cur = J.str(in, "currentPassword");
            if (cur == null || !n.verifyPassword(cur, old)) throw ApiEx.unauth("Неверный текущий пароль");
        }
        n.exec("UPDATE users SET password_hash=? WHERE id=?", n.hashPassword(newp), uid);
        return okTrue();
    }

    private JSONArray adminUsers(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT user_id FROM user_workspaces WHERE workspace_id=?", ws)) {
            while (c.moveToNext()) {
                JSONObject u = j.userPublic(c.getLong(0));
                if (u != null) a.put(u);
            }
        }
        return a;
    }

    private JSONObject adminUserCreate(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        String name = J.str(in, "fullName");
        String phone = J.str(in, "phone");
        if (name == null || phone == null) throw ApiEx.bad("Имя и телефон");
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        long id = n.insert("INSERT INTO users (full_name,position,phone,status,role_rights,created_at,guid,checkout_policy) VALUES (?,?,?,'invited',?,?,?,?)",
                name, J.str(in, "position"), phone, NodeDb.DEFAULT_RIGHTS, NodeDb.now(), NodeDb.guid(), NodeDb.DEFAULT_POLICY);
        n.exec("INSERT INTO user_workspaces (user_id, workspace_id) VALUES (?,?)", id, ws);
        return j.userPublic(id);
    }

    private JSONObject adminUserUpdate(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long id = J.lng(in, "id");
        n.exec("UPDATE users SET full_name=COALESCE(?,full_name), position=COALESCE(?,position), phone=COALESCE(?,phone), status=COALESCE(?,status), role_rights=COALESCE(?,role_rights) WHERE id=?",
                J.str(in, "fullName"), J.str(in, "position"), J.str(in, "phone"), J.str(in, "status"),
                in.has("roleRights") && in.optJSONObject("roleRights") != null ? in.optJSONObject("roleRights").toString() : null, id);
        return j.userPublic(id);
    }

    private JSONObject adminUserInvite(JSONObject in, Long uid) throws ApiEx {
        return wsCreateInvite(in, uid);
    }

    private JSONObject wsCreate(JSONObject in) {
        String name = J.str(in, "name");
        if (name == null) name = "Группа";
        String sync = J.str(in, "syncUrl");
        long id = n.insert("INSERT INTO workspaces (name,timezone,internal_id_prefix,comment,created_at,sync_url,guid) VALUES (?,?,?,?,?,?,?)",
                name, J.str(in, "timezone") == null ? "Europe/Moscow" : J.str(in, "timezone"),
                J.str(in, "internalIdPrefix") == null ? "ВН-" : J.str(in, "internalIdPrefix"),
                J.str(in, "comment"), NodeDb.now(), sync, NodeDb.guid());
        n.seedWorkspaceStatuses(id);
        if (sync != null) gossip.addPeer(sync, "relay", null);
        return j.workspaceJson(id);
    }

    private JSONObject wsUpdate(JSONObject in) throws ApiEx {
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        n.exec("UPDATE workspaces SET name=COALESCE(?,name), timezone=COALESCE(?,timezone), internal_id_prefix=COALESCE(?,internal_id_prefix), comment=?, sync_url=COALESCE(?,sync_url) WHERE id=?",
                J.str(in, "name"), J.str(in, "timezone"), J.str(in, "internalIdPrefix"), J.str(in, "comment"), J.str(in, "syncUrl"), id);
        if (J.str(in, "syncUrl") != null) gossip.addPeer(J.str(in, "syncUrl"), "relay", null);
        return j.workspaceJson(id);
    }

    private JSONObject wsCreateInvite(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        String token = NodeDb.guid();
        long max = J.lng(in, "maxUses") == null ? 20 : J.lng(in, "maxUses");
        n.exec("INSERT INTO invites (workspace_id,token,role,created_by,max_uses,created_at) VALUES (?,?,?,?,?,?)",
                ws, token, J.str(in, "role") == null ? "member" : J.str(in, "role"), uid, max, NodeDb.now());
        JSONObject wsj = j.workspaceJson(ws);
        JSONObject payload = J.obj();
        J.put(payload, "v", 1);
        J.put(payload, "t", "join");
        J.put(payload, "ws", ws);
        J.put(payload, "token", token);
        J.put(payload, "name", wsj == null ? null : wsj.opt("name"));
        J.put(payload, "server", gossip.lanOrigin());
        JSONObject o = J.obj();
        J.put(o, "token", token);
        J.put(o, "workspaceId", ws);
        J.put(o, "workspace", wsj);
        J.put(o, "payload", payload);
        return o;
    }

    private JSONArray wsInvites(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, token, role, max_uses, used_count, revoked, created_at FROM invites WHERE workspace_id=? AND revoked=0 ORDER BY id DESC", ws)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "token", c.getString(1));
                J.put(o, "role", c.getString(2));
                J.put(o, "maxUses", c.getLong(3));
                J.put(o, "usedCount", c.getLong(4));
                J.put(o, "revoked", c.getLong(5) != 0);
                J.put(o, "createdAt", c.getString(6));
                a.put(o);
            }
        }
        return a;
    }

    private JSONArray storagesList(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id FROM storages WHERE workspace_id=?", ws)) {
            while (c.moveToNext()) {
                JSONObject v = j.storageObj(c.getLong(0));
                if (v != null) {
                    if (!v.isNull("responsibleUserId")) J.put(v, "responsible", j.userPublic(v.optLong("responsibleUserId")));
                    a.put(v);
                }
            }
        }
        return a;
    }

    private JSONObject storageCreate(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        String name = J.str(in, "name");
        if (name == null) name = "Склад";
        long id = n.insert("INSERT INTO storages (name,responsible_user_id,workspace_id,address) VALUES (?,?,?,?)",
                name, J.lng(in, "responsibleUserId"), ws, J.str(in, "address"));
        return j.storageObj(id);
    }

    private JSONObject storageUpdate(JSONObject in) throws ApiEx {
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        n.exec("UPDATE storages SET name=COALESCE(?,name), responsible_user_id=?, address=COALESCE(?,address) WHERE id=?",
                J.str(in, "name"), J.lng(in, "responsibleUserId"), J.str(in, "address"), id);
        return j.storageObj(id);
    }

    private JSONArray sitesList(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, name, responsible_user_id, workspace_id FROM building_sites WHERE workspace_id=?", ws)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                Long u = c.isNull(2) ? null : c.getLong(2);
                J.put(o, "id", c.getLong(0));
                J.put(o, "name", c.getString(1));
                J.put(o, "responsibleUserId", u);
                J.put(o, "workspaceId", c.getLong(3));
                J.put(o, "responsible", userSafe(u));
                a.put(o);
            }
        }
        return a;
    }

    private JSONObject siteCreate(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        String name = J.str(in, "name");
        if (name == null) name = "Объект";
        long id = n.insert("INSERT INTO building_sites (name,responsible_user_id,workspace_id) VALUES (?,?,?)", name, J.lng(in, "responsibleUserId"), ws);
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "name", name);
        J.put(o, "workspaceId", ws);
        J.put(o, "responsibleUserId", J.lng(in, "responsibleUserId"));
        return o;
    }

    private JSONObject siteUpdate(JSONObject in) throws ApiEx {
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        n.exec("UPDATE building_sites SET name=COALESCE(?,name), responsible_user_id=? WHERE id=?", J.str(in, "name"), J.lng(in, "responsibleUserId"), id);
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "name", J.str(in, "name"));
        return o;
    }

    private String dictTable(String kind) throws ApiEx {
        if ("brands".equals(kind)) return "brands";
        if ("statuses".equals(kind)) return "statuses";
        if (kind == null || "categories".equals(kind)) return "categories";
        throw ApiEx.bad("kind");
    }

    private JSONArray dictList(JSONObject in, Long uid) throws ApiEx {
        String table = dictTable(J.str(in, "kind"));
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        String sql = "statuses".equals(table)
                ? "SELECT id, name, description, workspace_id, type, slug, color, bg FROM statuses WHERE workspace_id=?"
                : "SELECT id, name, description, workspace_id, type FROM " + table + " WHERE workspace_id=?";
        try (Cursor c = n.q(sql, ws)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "name", c.getString(1));
                J.put(o, "description", c.isNull(2) ? null : c.getString(2));
                J.put(o, "workspaceId", c.getLong(3));
                J.put(o, "type", c.getString(4));
                if ("statuses".equals(table)) {
                    J.put(o, "slug", c.getString(5));
                    J.put(o, "color", c.getString(6));
                    J.put(o, "bg", c.getString(7));
                }
                a.put(o);
            }
        }
        return a;
    }

    private JSONObject dictCreate(JSONObject in, Long uid) throws ApiEx {
        String table = dictTable(J.str(in, "kind"));
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        String name = J.str(in, "name");
        if (name == null) throw ApiEx.bad("name");
        long id;
        if ("statuses".equals(table)) {
            id = n.insert("INSERT INTO statuses (name,description,workspace_id,type,slug,color,bg) VALUES (?,?,?,'status',?,?,?)",
                    name, J.str(in, "description"), ws, J.str(in, "slug") == null ? "custom" : J.str(in, "slug"),
                    J.str(in, "color") == null ? "#5E629B" : J.str(in, "color"),
                    J.str(in, "bg") == null ? "#EDEDF7" : J.str(in, "bg"));
        } else {
            String ty = "brands".equals(table) ? "brand" : "category";
            id = n.insert("INSERT INTO " + table + " (name,description,workspace_id,type) VALUES (?,?,?,?)", name, J.str(in, "description"), ws, ty);
        }
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "name", name);
        J.put(o, "workspaceId", ws);
        return o;
    }

    private JSONObject dictUpdate(JSONObject in) throws ApiEx {
        String table = dictTable(J.str(in, "kind"));
        Long id = J.lng(in, "id");
        if (id == null) throw ApiEx.bad("id");
        n.exec("UPDATE " + table + " SET name=COALESCE(?,name), description=? WHERE id=?", J.str(in, "name"), J.str(in, "description"), id);
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "name", J.str(in, "name"));
        return o;
    }

    private JSONObject dictRemove(JSONObject in) throws ApiEx {
        String table = dictTable(J.str(in, "kind"));
        n.exec("DELETE FROM " + table + " WHERE id=?", J.lng(in, "id"));
        return okTrue();
    }

    private JSONObject reportFault(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long itemId = J.lng(in, "itemId");
        String desc = J.str(in, "description");
        if (itemId == null || desc == null) throw ApiEx.bad("itemId и описание");
        JSONObject item = j.itemJson(itemId, false);
        long ws = item == null ? n.wsFallback(uid) : item.optLong("workspaceId", 1);
        long id = n.insert("INSERT INTO faults (item_id,workspace_id,author_id,severity,description,photo_url,created_at) VALUES (?,?,?,?,?,?,?)",
                itemId, ws, uid, J.str(in, "severity") == null ? "medium" : J.str(in, "severity"), desc, J.str(in, "photoUrl"), NodeDb.now());
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "status", "open");
        return o;
    }

    private JSONArray listFaults(JSONObject in) {
        Long itemId = J.lng(in, "itemId");
        JSONArray a = J.arr();
        String sql = itemId == null ? "SELECT id,item_id,author_id,severity,description,status,created_at FROM faults ORDER BY id DESC LIMIT 100"
                : "SELECT id,item_id,author_id,severity,description,status,created_at FROM faults WHERE item_id=? ORDER BY id DESC";
        try (Cursor c = itemId == null ? n.q(sql) : n.q(sql, itemId)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "itemId", c.getLong(1));
                J.put(o, "authorId", c.getLong(2));
                J.put(o, "severity", c.getString(3));
                J.put(o, "description", c.getString(4));
                J.put(o, "status", c.getString(5));
                J.put(o, "createdAt", c.getString(6));
                J.put(o, "author", j.userPublic(c.getLong(2)));
                a.put(o);
            }
        }
        return a;
    }

    private JSONObject resolveFault(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long id = J.lng(in, "id");
        n.exec("UPDATE faults SET status='resolved', resolution=?, resolver_id=?, resolved_at=? WHERE id=?", J.str(in, "resolution"), uid, NodeDb.now(), id);
        return okTrue();
    }

    private JSONObject requestChange(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long itemId = J.lng(in, "itemId");
        if (itemId == null) throw ApiEx.bad("itemId");
        JSONObject item = j.itemJson(itemId, false);
        long ws = item == null ? n.wsFallback(uid) : item.optLong("workspaceId", 1);
        JSONObject payload = in.optJSONObject("payload");
        long id = n.insert("INSERT INTO change_requests (item_id,workspace_id,author_id,payload,comment,created_at) VALUES (?,?,?,?,?,?)",
                itemId, ws, uid, payload == null ? "{}" : payload.toString(), J.str(in, "comment"), NodeDb.now());
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "status", "pending");
        return o;
    }

    private JSONArray listChanges(JSONObject in) {
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id,item_id,author_id,payload,comment,status,created_at FROM change_requests ORDER BY id DESC LIMIT 100")) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "itemId", c.getLong(1));
                J.put(o, "authorId", c.getLong(2));
                try { J.put(o, "payload", new JSONObject(c.getString(3))); } catch (Exception e) { J.put(o, "payload", c.getString(3)); }
                J.put(o, "comment", c.isNull(4) ? null : c.getString(4));
                J.put(o, "status", c.getString(5));
                J.put(o, "createdAt", c.getString(6));
                a.put(o);
            }
        }
        return a;
    }

    private JSONObject decideChange(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        Long id = J.lng(in, "id");
        String status = Boolean.TRUE.equals(J.bool(in, "approve")) ? "approved" : "rejected";
        n.exec("UPDATE change_requests SET status=?, reason=?, decided_by=?, decided_at=? WHERE id=?", status, J.str(in, "reason"), uid, NodeDb.now(), id);
        return okTrue();
    }

    private JSONArray chatList(JSONObject in, Long uid) {
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id,workspace_id,user_id,text,created_at FROM chat_messages WHERE workspace_id=? ORDER BY id DESC LIMIT 100", ws)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "workspaceId", c.getLong(1));
                J.put(o, "userId", c.getLong(2));
                J.put(o, "text", c.getString(3));
                J.put(o, "createdAt", c.getString(4));
                J.put(o, "user", j.userPublic(c.getLong(2)));
                a.put(o);
            }
        }
        return a;
    }

    private JSONObject chatSend(JSONObject in, Long uid) throws ApiEx {
        n.requireUser(uid);
        String text = J.str(in, "text");
        if (text == null) throw ApiEx.bad("text");
        long ws = J.lng(in, "workspaceId") != null ? J.lng(in, "workspaceId") : n.wsFallback(uid);
        long id = n.insert("INSERT INTO chat_messages (workspace_id,user_id,text,created_at) VALUES (?,?,?,?)", ws, uid, text, NodeDb.now());
        JSONObject o = J.obj();
        J.put(o, "id", id);
        J.put(o, "workspaceId", ws);
        J.put(o, "userId", uid);
        J.put(o, "text", text);
        J.put(o, "createdAt", NodeDb.now());
        J.put(o, "user", j.userPublic(uid));
        return o;
    }

    private String nextTransferCode(long ws) {
        long cnt = n.long1("SELECT COUNT(*) FROM transfers WHERE workspace_id=?", ws);
        return "ПП-" + String.format(java.util.Locale.US, "%04d", cnt + 1);
    }

    private String nameOf(Long uid) {
        JSONObject u = userSafe(uid);
        return u == null ? "" : u.optString("fullName", "");
    }

    private JSONObject userSafe(Long uid) {
        return uid == null ? null : j.userPublic(uid);
    }
}
