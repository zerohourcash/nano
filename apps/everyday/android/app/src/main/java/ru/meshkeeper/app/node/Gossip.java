package ru.meshkeeper.app.node;

import android.database.Cursor;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.NetworkInterface;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;

final class Gossip {
    private static final String TAG = "MeshKeeperGossip";
    private final NodeDb n;
    private final JsonShapes j;
    volatile String lanOrigin = "http://127.0.0.1:8765";
    private final AtomicBoolean running = new AtomicBoolean(false);
    private ExecutorService pool;

    Gossip(NodeDb n, JsonShapes j) {
        this.n = n;
        this.j = j;
    }

    String lanOrigin() { return lanOrigin; }

    void start(String optionalRelay) {
        lanOrigin = "http://127.0.0.1:" + NodeRuntime.PORT;
        if (optionalRelay != null && !optionalRelay.isEmpty()) addPeer(optionalRelay, "relay", null);
        if (running.compareAndSet(false, true)) {
            pool = Executors.newSingleThreadExecutor();
            pool.execute(this::loop);
        }
    }

    void stop() {
        running.set(false);
        if (pool != null) pool.shutdownNow();
    }

    JSONObject hello() {
        JSONObject o = J.obj();
        J.put(o, "ok", true);
        J.put(o, "nodeId", n.kvGet("node_id"));
        J.put(o, "pubkey", n.kvGet("node_pk"));
        J.put(o, "name", n.kvGet("node_name"));
        J.put(o, "protocol", "meshkeeper-sync/1");
        J.put(o, "ledger", "sha256");
        J.put(o, "lanUrl", lanOrigin);
        return o;
    }

    JSONObject status() {
        JSONObject o = J.obj();
        J.put(o, "nodeId", n.kvGet("node_id"));
        J.put(o, "name", n.kvGet("node_name"));
        J.put(o, "pubkey", n.kvGet("node_pk"));
        J.put(o, "url", lanOrigin);
        J.put(o, "localUrl", "http://127.0.0.1:" + NodeRuntime.PORT);
        J.put(o, "peers", listPeers());
        J.put(o, "openConflicts", n.long1("SELECT COUNT(*) FROM conflicts WHERE status='open'"));
        return o;
    }

    JSONArray listPeers() {
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, node_id, url, name, last_seen, last_sync, last_error FROM peers ORDER BY id DESC")) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "nodeId", c.isNull(1) ? null : c.getString(1));
                J.put(o, "url", c.getString(2));
                J.put(o, "name", c.isNull(3) ? null : c.getString(3));
                J.put(o, "lastSeen", c.isNull(4) ? null : c.getString(4));
                J.put(o, "lastSync", c.isNull(5) ? null : c.getString(5));
                J.put(o, "lastError", c.isNull(6) ? null : c.getString(6));
                a.put(o);
            }
        }
        return a;
    }

    JSONObject addPeer(String url, String name, String nodeId) {
        String clean = url == null ? "" : url.trim().replaceAll("/+$", "");
        if (clean.isEmpty()) return J.obj();
        Long existing = n.longN("SELECT id FROM peers WHERE url=?", clean);
        if (existing == null) {
            n.exec("INSERT INTO peers (url, name, node_id, last_seen) VALUES (?,?,?,?)", clean, name, nodeId, NodeDb.now());
        } else {
            n.exec("UPDATE peers SET name=COALESCE(?,name), node_id=COALESCE(?,node_id), last_seen=? WHERE url=?", name, nodeId, NodeDb.now(), clean);
        }
        JSONObject o = J.obj();
        J.put(o, "ok", true);
        J.put(o, "url", clean);
        return o;
    }

    JSONArray listConflicts() {
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, workspace_id, item_id, item_guid, status, description, left_label, right_label, created_at FROM conflicts ORDER BY id DESC LIMIT 200")) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                Long itemId = c.isNull(2) ? null : c.getLong(2);
                J.put(o, "id", c.getLong(0));
                J.put(o, "workspaceId", c.isNull(1) ? null : c.getLong(1));
                J.put(o, "itemId", itemId);
                J.put(o, "itemGuid", c.isNull(3) ? null : c.getString(3));
                J.put(o, "status", c.getString(4));
                J.put(o, "description", c.getString(5));
                J.put(o, "leftLabel", c.isNull(6) ? null : c.getString(6));
                J.put(o, "rightLabel", c.isNull(7) ? null : c.getString(7));
                J.put(o, "createdAt", c.getString(8));
                J.put(o, "item", itemId == null ? null : j.itemJson(itemId, false));
                a.put(o);
            }
        }
        return a;
    }

    JSONObject resolveConflict(long id, Long responsible, long uid) {
        Long itemId = n.longN("SELECT item_id FROM conflicts WHERE id=?", id);
        Long ws = n.longN("SELECT workspace_id FROM conflicts WHERE id=?", id);
        n.exec("UPDATE conflicts SET status='resolved', resolved_at=?, resolver_id=? WHERE id=?", NodeDb.now(), uid, id);
        if (itemId != null && ws != null) {
            String slug = responsible != null ? "in-work" : "in-stock";
            Long st = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug=?", ws, slug);
            n.exec("UPDATE items SET responsible_user_id=?, status_id=COALESCE(?,status_id) WHERE id=?", responsible, st, itemId);
            n.appendLedger(ws, uid, itemId, "update", null, null, null, "Конфликт выдачи разрешён");
        }
        return helloOk();
    }

    JSONObject exportJournal() {
        JSONObject o = J.obj();
        J.put(o, "v", 1);
        J.put(o, "nodeId", n.kvGet("node_id"));
        J.put(o, "nodeName", n.kvGet("node_name"));
        J.put(o, "nodePubkey", n.kvGet("node_pk"));
        J.put(o, "exportedAt", NodeDb.now());
        JSONArray workspaces = J.arr();
        try (Cursor c = n.q("SELECT id, name, timezone, internal_id_prefix, comment, guid, sync_url FROM workspaces")) {
            while (c.moveToNext()) {
                JSONObject w = J.obj();
                J.put(w, "id", c.getLong(0));
                J.put(w, "name", c.getString(1));
                J.put(w, "timezone", c.getString(2));
                J.put(w, "internalIdPrefix", c.getString(3));
                J.put(w, "comment", c.isNull(4) ? null : c.getString(4));
                J.put(w, "guid", c.isNull(5) ? null : c.getString(5));
                J.put(w, "syncUrl", c.isNull(6) ? null : c.getString(6));
                workspaces.put(w);
            }
        }
        J.put(o, "workspaces", workspaces);
        JSONArray users = J.arr();
        try (Cursor c = n.q("SELECT id, full_name, position, phone, status, role_rights, checkout_policy, pubkey, guid FROM users")) {
            while (c.moveToNext()) {
                JSONObject u = J.obj();
                J.put(u, "id", c.getLong(0));
                J.put(u, "fullName", c.getString(1));
                J.put(u, "position", c.isNull(2) ? null : c.getString(2));
                J.put(u, "phone", c.getString(3));
                J.put(u, "status", c.getString(4));
                J.put(u, "roleRights", c.isNull(5) ? null : c.getString(5));
                J.put(u, "checkoutPolicy", c.isNull(6) ? null : c.getString(6));
                J.put(u, "pubkey", c.isNull(7) ? null : c.getString(7));
                J.put(u, "guid", c.isNull(8) ? null : c.getString(8));
                users.put(u);
            }
        }
        J.put(o, "users", users);
        JSONArray items = J.arr();
        try (Cursor c = n.q("SELECT id, internal_id, title, status_id, responsible_user_id, workspace_id, serial_number, qr_code, due_at, guid, calibrated_until, min_quantity, quantitative, quantity, unit, cost, comment FROM items")) {
            while (c.moveToNext()) {
                long id = c.getLong(0);
                Long st = c.isNull(3) ? null : c.getLong(3);
                Long resp = c.isNull(4) ? null : c.getLong(4);
                long ws = c.getLong(5);
                JSONObject it = J.obj();
                J.put(it, "guid", c.isNull(9) ? null : c.getString(9));
                J.put(it, "internalId", c.getString(1));
                J.put(it, "title", c.getString(2));
                J.put(it, "workspaceGuid", guidOf("workspaces", ws));
                J.put(it, "responsibleGuid", resp == null ? null : guidOf("users", resp));
                J.put(it, "serialNumber", c.isNull(6) ? null : c.getString(6));
                J.put(it, "qrCode", c.isNull(7) ? null : c.getString(7));
                J.put(it, "dueAt", c.isNull(8) ? null : c.getString(8));
                J.put(it, "calibratedUntil", c.isNull(10) ? null : c.getString(10));
                J.put(it, "minQuantity", c.isNull(11) ? null : c.getDouble(11));
                J.put(it, "quantitative", c.getLong(12) != 0);
                J.put(it, "quantity", c.isNull(13) ? null : c.getDouble(13));
                J.put(it, "unit", c.isNull(14) ? null : c.getString(14));
                J.put(it, "cost", c.isNull(15) ? null : c.getDouble(15));
                J.put(it, "comment", c.isNull(16) ? null : c.getString(16));
                J.put(it, "statusSlug", st == null ? "in-stock" : n.str1("SELECT slug FROM statuses WHERE id=?", st));
                J.put(it, "localId", id);
                items.put(it);
            }
        }
        J.put(o, "items", items);
        JSONArray history = J.arr();
        try (Cursor c = n.q("SELECT workspace_id, item_id, type, actor_user_id, from_label, to_label, quantity_delta, comment, prev_hash, hash, signature, pubkey, created_at, guid FROM history_entries ORDER BY id")) {
            while (c.moveToNext()) {
                JSONObject h = J.obj();
                J.put(h, "workspaceGuid", guidOf("workspaces", c.getLong(0)));
                J.put(h, "itemGuid", c.isNull(1) ? null : guidOf("items", c.getLong(1)));
                J.put(h, "type", c.getString(2));
                J.put(h, "actorGuid", guidOf("users", c.getLong(3)));
                J.put(h, "fromLabel", c.isNull(4) ? null : c.getString(4));
                J.put(h, "toLabel", c.isNull(5) ? null : c.getString(5));
                J.put(h, "quantityDelta", c.isNull(6) ? null : c.getDouble(6));
                J.put(h, "comment", c.isNull(7) ? null : c.getString(7));
                J.put(h, "prevHash", c.isNull(8) ? null : c.getString(8));
                J.put(h, "hash", c.getString(9));
                J.put(h, "signature", c.isNull(10) ? null : c.getString(10));
                J.put(h, "pubkey", c.isNull(11) ? null : c.getString(11));
                J.put(h, "createdAt", c.getString(12));
                J.put(h, "guid", c.isNull(13) ? null : c.getString(13));
                history.put(h);
            }
        }
        J.put(o, "history", history);
        JSONArray invites = J.arr();
        try (Cursor c = n.q("SELECT token, workspace_id, role, max_uses, used_count, revoked, created_at FROM invites")) {
            while (c.moveToNext()) {
                JSONObject inv = J.obj();
                J.put(inv, "token", c.getString(0));
                J.put(inv, "workspaceGuid", guidOf("workspaces", c.getLong(1)));
                J.put(inv, "role", c.getString(2));
                J.put(inv, "maxUses", c.getLong(3));
                J.put(inv, "usedCount", c.getLong(4));
                J.put(inv, "revoked", c.getLong(5) != 0);
                J.put(inv, "createdAt", c.getString(6));
                invites.put(inv);
            }
        }
        J.put(o, "invites", invites);
        JSONArray memberships = J.arr();
        try (Cursor c = n.q("SELECT user_id, workspace_id FROM user_workspaces")) {
            while (c.moveToNext()) {
                JSONObject m = J.obj();
                J.put(m, "userGuid", guidOf("users", c.getLong(0)));
                J.put(m, "workspaceGuid", guidOf("workspaces", c.getLong(1)));
                memberships.put(m);
            }
        }
        J.put(o, "memberships", memberships);
        JSONArray holdings = J.arr();
        try (Cursor c = n.q("SELECT item_id, user_id, quantity, due_at, comment, created_at, returned_at FROM item_holdings")) {
            while (c.moveToNext()) {
                JSONObject h = J.obj();
                J.put(h, "itemGuid", guidOf("items", c.getLong(0)));
                J.put(h, "userGuid", guidOf("users", c.getLong(1)));
                J.put(h, "quantity", c.getDouble(2));
                J.put(h, "dueAt", c.isNull(3) ? null : c.getString(3));
                J.put(h, "comment", c.isNull(4) ? null : c.getString(4));
                J.put(h, "createdAt", c.getString(5));
                J.put(h, "returnedAt", c.isNull(6) ? null : c.getString(6));
                holdings.put(h);
            }
        }
        J.put(o, "holdings", holdings);
        return o;
    }

    JSONObject importJournal(JSONObject journal) {
        int workspaces = 0, users = 0, itemsN = 0, ops = 0, skipped = 0, conflicts = 0, invitesN = 0;
        JSONArray wsA = journal.optJSONArray("workspaces");
        if (wsA != null) {
            for (int i = 0; i < wsA.length(); i++) {
                upsertWorkspace(wsA.optJSONObject(i));
                workspaces++;
            }
        }
        JSONArray uA = journal.optJSONArray("users");
        if (uA != null) {
            for (int i = 0; i < uA.length(); i++) {
                upsertUser(uA.optJSONObject(i));
                users++;
            }
        }
        JSONArray iA = journal.optJSONArray("items");
        if (iA != null) {
            for (int i = 0; i < iA.length(); i++) {
                JSONObject it = iA.optJSONObject(i);
                if (it == null) continue;
                String guid = it.optString("guid", "");
                String wsG = it.optString("workspaceGuid", "");
                Long ws = idByGuid("workspaces", wsG);
                if (ws == null) continue;
                Long resp = idByGuid("users", it.optString("responsibleGuid", ""));
                String slug = it.optString("statusSlug", "in-stock");
                Long st = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug=?", ws, slug);
                if (st == null) {
                    n.seedWorkspaceStatuses(ws);
                    st = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug=?", ws, slug);
                }
                Long local = guid.isEmpty() ? null : idByGuid("items", guid);
                if (local != null) {
                    Long localResp = n.longN("SELECT responsible_user_id FROM items WHERE id=?", local);
                    if (localResp != null && resp != null && !localResp.equals(resp)) {
                        String desc = "Двое взяли один предмет офлайн";
                        n.exec("INSERT INTO conflicts (workspace_id,item_id,item_guid,description,left_label,right_label,created_at) VALUES (?,?,?,?,?,?,?)",
                                ws, local, guid, desc, "user:" + localResp, "user:" + resp, NodeDb.now());
                        Long check = n.longN("SELECT id FROM statuses WHERE workspace_id=? AND slug='needs-check'", ws);
                        n.exec("UPDATE items SET responsible_user_id=NULL, status_id=COALESCE(?,status_id) WHERE id=?", check, local);
                        conflicts++;
                    } else {
                        n.exec("UPDATE items SET due_at=COALESCE(?,due_at), responsible_user_id=? WHERE id=?", it.optString("dueAt", null), resp, local);
                    }
                    itemsN++;
                    continue;
                }
                n.insert("INSERT INTO items (internal_id,title,status_id,responsible_user_id,workspace_id,serial_number,qr_code,due_at,guid,calibrated_until,min_quantity,quantitative,quantity,unit,cost,comment,created_at) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                        it.optString("internalId", "ВН-0000"), it.optString("title", "Инструмент"), st, resp, ws,
                        emptyToNull(it.optString("serialNumber", "")), emptyToNull(it.optString("qrCode", "")),
                        emptyToNull(it.optString("dueAt", "")), guid.isEmpty() ? NodeDb.guid() : guid,
                        emptyToNull(it.optString("calibratedUntil", "")), it.has("minQuantity") && !it.isNull("minQuantity") ? it.optDouble("minQuantity") : null,
                        it.optBoolean("quantitative") ? 1 : 0, it.has("quantity") && !it.isNull("quantity") ? it.optDouble("quantity") : null,
                        emptyToNull(it.optString("unit", "")), it.has("cost") && !it.isNull("cost") ? it.optDouble("cost") : null,
                        emptyToNull(it.optString("comment", "")), NodeDb.now());
                itemsN++;
            }
        }
        JSONArray hA = journal.optJSONArray("history");
        if (hA != null) {
            for (int i = 0; i < hA.length(); i++) {
                JSONObject h = hA.optJSONObject(i);
                if (h == null) continue;
                String hash = h.optString("hash", "");
                if (hash.isEmpty()) { skipped++; continue; }
                if (n.long1("SELECT COUNT(*) FROM history_entries WHERE hash=?", hash) > 0) { skipped++; continue; }
                Long ws = idByGuid("workspaces", h.optString("workspaceGuid", ""));
                if (ws == null) { skipped++; continue; }
                Long item = idByGuid("items", h.optString("itemGuid", ""));
                Long actor = idByGuid("users", h.optString("actorGuid", ""));
                if (actor == null) actor = 1L;
                try {
                    n.exec("INSERT INTO history_entries (workspace_id,item_id,type,actor_user_id,from_label,to_label,quantity_delta,comment,prev_hash,hash,signature,pubkey,created_at,guid) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                            ws, item, h.optString("type", "update"), actor,
                            emptyToNull(h.optString("fromLabel", "")), emptyToNull(h.optString("toLabel", "")),
                            h.has("quantityDelta") && !h.isNull("quantityDelta") ? h.optDouble("quantityDelta") : null,
                            emptyToNull(h.optString("comment", "")), emptyToNull(h.optString("prevHash", "")), hash,
                            emptyToNull(h.optString("signature", "")), emptyToNull(h.optString("pubkey", "")),
                            h.optString("createdAt", NodeDb.now()), emptyToNull(h.optString("guid", "")));
                    ops++;
                } catch (Exception e) { skipped++; }
            }
        }
        JSONArray invA = journal.optJSONArray("invites");
        if (invA != null) {
            for (int i = 0; i < invA.length(); i++) {
                JSONObject inv = invA.optJSONObject(i);
                if (inv == null) continue;
                String token = inv.optString("token", "");
                Long ws = idByGuid("workspaces", inv.optString("workspaceGuid", ""));
                if (token.isEmpty() || ws == null) continue;
                if (n.long1("SELECT COUNT(*) FROM invites WHERE token=?", token) == 0) {
                    n.exec("INSERT INTO invites (workspace_id,token,role,max_uses,used_count,revoked,created_at) VALUES (?,?,?,?,?,?,?)",
                            ws, token, inv.optString("role", "member"), inv.optLong("maxUses", 20),
                            inv.optLong("usedCount", 0), inv.optBoolean("revoked") ? 1 : 0, inv.optString("createdAt", NodeDb.now()));
                }
                invitesN++;
            }
        }
        JSONArray memA = journal.optJSONArray("memberships");
        if (memA != null) {
            for (int i = 0; i < memA.length(); i++) {
                JSONObject m = memA.optJSONObject(i);
                if (m == null) continue;
                Long user = idByGuid("users", m.optString("userGuid", ""));
                Long ws = idByGuid("workspaces", m.optString("workspaceGuid", ""));
                if (user == null || ws == null) continue;
                if (n.long1("SELECT COUNT(*) FROM user_workspaces WHERE user_id=? AND workspace_id=?", user, ws) == 0) {
                    n.exec("INSERT INTO user_workspaces (user_id, workspace_id) VALUES (?,?)", user, ws);
                }
            }
        }
        JSONArray holdA = journal.optJSONArray("holdings");
        if (holdA != null) {
            for (int i = 0; i < holdA.length(); i++) {
                JSONObject h = holdA.optJSONObject(i);
                if (h == null) continue;
                Long item = idByGuid("items", h.optString("itemGuid", ""));
                Long user = idByGuid("users", h.optString("userGuid", ""));
                if (item == null || user == null) continue;
                String created = h.optString("createdAt", "");
                if (n.long1("SELECT COUNT(*) FROM item_holdings WHERE item_id=? AND user_id=? AND created_at=?", item, user, created) == 0) {
                    n.exec("INSERT INTO item_holdings (item_id,user_id,quantity,due_at,comment,created_at,returned_at) VALUES (?,?,?,?,?,?,?)",
                            item, user, h.optDouble("quantity", 1), emptyToNull(h.optString("dueAt", "")),
                            emptyToNull(h.optString("comment", "")), created.isEmpty() ? NodeDb.now() : created,
                            emptyToNull(h.optString("returnedAt", "")));
                }
            }
        }
        JSONObject out = J.obj();
        J.put(out, "workspaces", workspaces);
        J.put(out, "users", users);
        J.put(out, "items", itemsN);
        J.put(out, "ops", ops);
        J.put(out, "skipped", skipped);
        J.put(out, "conflicts", conflicts);
        J.put(out, "invites", invitesN);
        return out;
    }

    JSONObject applyRemote(JSONObject journal, String peerUrl) {
        JSONObject r = importJournal(journal);
        addPeer(peerUrl, journal.optString("nodeName", null), journal.optString("nodeId", null));
        n.exec("UPDATE peers SET last_sync=?, last_error=NULL WHERE url=?", NodeDb.now(), peerUrl.replaceAll("/+$", ""));
        return r;
    }

    JSONObject pullOne(String url) {
        String clean = url.trim().replaceAll("/+$", "");
        try {
            JSONObject hello = httpGetJson(clean + "/sync/hello");
            if (hello == null || !hello.optBoolean("ok", false)) {
                n.exec("UPDATE peers SET last_error=? WHERE url=?", "нет /sync/hello", clean);
                JSONObject e = J.obj();
                J.put(e, "ok", false);
                J.put(e, "error", "Узел не отвечает: " + clean);
                return e;
            }
            JSONObject journal = httpGetJson(clean + "/sync/journal");
            if (journal == null) {
                JSONObject e = J.obj();
                J.put(e, "ok", false);
                J.put(e, "error", "Нет журнала");
                return e;
            }
            JSONObject r = applyRemote(journal, clean);
            httpPostJson(clean + "/sync/journal", exportJournal());
            J.put(r, "ok", true);
            return r;
        } catch (Exception e) {
            n.exec("UPDATE peers SET last_error=? WHERE url=?", e.getMessage(), clean);
            JSONObject o = J.obj();
            J.put(o, "ok", false);
            J.put(o, "error", e.getMessage());
            return o;
        }
    }

    private void loop() {
        int ticks = 0;
        while (running.get()) {
            ticks++;
            try {
                List<String> peers = new ArrayList<>();
                try (Cursor c = n.q("SELECT url FROM peers")) {
                    while (c.moveToNext()) peers.add(c.getString(0));
                }
                JSONObject mine = exportJournal();
                for (String url : peers) {
                    if (url.equals(lanOrigin) || url.contains("127.0.0.1")) continue;
                    try {
                        JSONObject hello = httpGetJson(url + "/sync/hello");
                        if (hello == null) {
                            n.exec("UPDATE peers SET last_error=? WHERE url=?", "offline", url);
                            continue;
                        }
                        JSONObject journal = httpGetJson(url + "/sync/journal");
                        if (journal != null) applyRemote(journal, url);
                        httpPostJson(url + "/sync/journal", mine);
                    } catch (Exception e) {
                        n.exec("UPDATE peers SET last_error=? WHERE url=?", e.getMessage(), url);
                    }
                }
                if (ticks % 4 == 1) lanProbe();
            } catch (Exception e) {
                Log.w(TAG, "gossip", e);
            }
            try { Thread.sleep(8000); } catch (InterruptedException ie) { return; }
        }
    }

    private void lanProbe() {
        String ip = lanIpv4();
        String[] p = ip.split("\\.");
        if (p.length != 4) return;
        String prefix = p[0] + "." + p[1] + "." + p[2];
        int[] last = {1, 2, 10, 20, 24, 50, 100, 101, 110, 120, 150, 200, 254};
        int[] ports = {8765, 8080};
        for (int oct : last) {
            String host = prefix + "." + oct;
            if (host.equals(ip)) continue;
            for (int port : ports) {
                String url = "http://" + host + ":" + port;
                try {
                    JSONObject hello = httpGetJson(url + "/sync/hello");
                    if (hello != null && hello.optBoolean("ok", false)) {
                        addPeer(url, hello.optString("name", "LAN"), hello.optString("nodeId", null));
                    }
                } catch (Exception ignored) {}
            }
        }
    }

    static String lanIpv4() {
        try {
            for (NetworkInterface ni : Collections.list(NetworkInterface.getNetworkInterfaces())) {
                if (!ni.isUp() || ni.isLoopback()) continue;
                for (InetAddress a : Collections.list(ni.getInetAddresses())) {
                    if (a instanceof Inet4Address && !a.isLoopbackAddress()) {
                        String h = a.getHostAddress();
                        if (h != null && (h.startsWith("192.168.") || h.startsWith("10.") || h.startsWith("172."))) return h;
                    }
                }
            }
        } catch (Exception ignored) {}
        return "127.0.0.1";
    }

    private String guidOf(String table, long id) {
        String g = n.str1("SELECT guid FROM " + table + " WHERE id=?", id);
        if (g == null || g.isEmpty()) {
            g = NodeDb.guid();
            n.exec("UPDATE " + table + " SET guid=? WHERE id=?", g, id);
        }
        return g;
    }

    private Long idByGuid(String table, String guid) {
        if (guid == null || guid.isEmpty()) return null;
        return n.longN("SELECT id FROM " + table + " WHERE guid=?", guid);
    }

    private void upsertWorkspace(JSONObject w) {
        if (w == null) return;
        String guid = w.optString("guid", "");
        if (!guid.isEmpty()) {
            Long id = n.longN("SELECT id FROM workspaces WHERE guid=?", guid);
            if (id != null) return;
        }
        long id = n.insert("INSERT INTO workspaces (name,timezone,internal_id_prefix,comment,created_at,guid,sync_url) VALUES (?,?,?,?,?,?,?)",
                w.optString("name", "Группа"), w.optString("timezone", "Europe/Moscow"),
                w.optString("internalIdPrefix", "ВН-"), emptyToNull(w.optString("comment", "")),
                NodeDb.now(), guid.isEmpty() ? NodeDb.guid() : guid, emptyToNull(w.optString("syncUrl", "")));
        n.seedWorkspaceStatuses(id);
    }

    private void upsertUser(JSONObject u) {
        if (u == null) return;
        String guid = u.optString("guid", "");
        String phone = u.optString("phone", "");
        String pk = u.optString("pubkey", "");
        if (!guid.isEmpty()) {
            if (n.longN("SELECT id FROM users WHERE guid=?", guid) != null) return;
        }
        if (!pk.isEmpty() && n.longN("SELECT id FROM users WHERE pubkey=?", pk) != null) return;
        if (!phone.isEmpty() && n.findUserPhone(phone) != null) return;
        n.insert("INSERT INTO users (full_name,position,phone,status,role_rights,checkout_policy,pubkey,privkey,guid,created_at) VALUES (?,?,?,?,?,?,?,?,?,?)",
                u.optString("fullName", "Участник"), emptyToNull(u.optString("position", "")),
                phone.isEmpty() ? ("sync-" + (guid.isEmpty() ? NodeDb.guid().substring(0, 8) : guid.substring(0, Math.min(8, guid.length())))) : phone,
                u.optString("status", "active"), emptyToNull(u.optString("roleRights", NodeDb.DEFAULT_RIGHTS)),
                emptyToNull(u.optString("checkoutPolicy", NodeDb.DEFAULT_POLICY)),
                pk.isEmpty() ? NodeDb.guid() : pk, NodeDb.guid(), guid.isEmpty() ? NodeDb.guid() : guid, NodeDb.now());
    }

    private static String emptyToNull(String s) {
        return s == null || s.isEmpty() ? null : s;
    }

    private JSONObject helloOk() {
        JSONObject o = J.obj();
        J.put(o, "ok", true);
        return o;
    }

    private JSONObject httpGetJson(String url) {
        HttpURLConnection c = null;
        try {
            c = (HttpURLConnection) new URL(url).openConnection();
            c.setConnectTimeout(2500);
            c.setReadTimeout(4000);
            c.setRequestMethod("GET");
            c.setRequestProperty("Accept", "application/json");
            int code = c.getResponseCode();
            if (code < 200 || code >= 300) return null;
            String body = read(c.getInputStream());
            if (body == null || body.isEmpty() || body.trim().startsWith("<")) return null;
            return new JSONObject(body);
        } catch (Exception e) {
            return null;
        } finally {
            if (c != null) c.disconnect();
        }
    }

    private void httpPostJson(String url, JSONObject body) {
        HttpURLConnection c = null;
        try {
            c = (HttpURLConnection) new URL(url).openConnection();
            c.setConnectTimeout(2500);
            c.setReadTimeout(4000);
            c.setRequestMethod("POST");
            c.setDoOutput(true);
            c.setRequestProperty("Content-Type", "application/json; charset=utf-8");
            byte[] data = body.toString().getBytes(StandardCharsets.UTF_8);
            try (OutputStream os = c.getOutputStream()) { os.write(data); }
            c.getResponseCode();
        } catch (Exception ignored) {
        } finally {
            if (c != null) c.disconnect();
        }
    }

    private static String read(InputStream in) throws Exception {
        BufferedReader br = new BufferedReader(new InputStreamReader(in, StandardCharsets.UTF_8));
        StringBuilder sb = new StringBuilder();
        String line;
        while ((line = br.readLine()) != null) sb.append(line);
        return sb.toString();
    }
}
