package ru.meshkeeper.app.node;

import android.database.Cursor;

import org.json.JSONArray;
import org.json.JSONObject;

final class JsonShapes {
    private final NodeDb n;

    JsonShapes(NodeDb n) { this.n = n; }

    JSONObject userPublic(Long id) {
        if (id == null) return null;
        try (Cursor c = n.q("SELECT id, full_name, position, phone, avatar_url, status, role_rights, pubkey, created_at, checkout_policy, guid FROM users WHERE id=?", id)) {
            if (!c.moveToFirst()) return null;
            JSONObject o = J.obj();
            J.put(o, "id", c.getLong(0));
            J.put(o, "fullName", c.getString(1));
            J.put(o, "position", c.isNull(2) ? null : c.getString(2));
            J.put(o, "phone", c.getString(3));
            J.put(o, "avatarUrl", c.isNull(4) ? null : c.getString(4));
            J.put(o, "status", c.getString(5));
            JSONObject rights;
            try {
                String raw = c.isNull(6) ? null : c.getString(6);
                rights = raw == null || raw.isEmpty() ? new JSONObject(NodeDb.DEFAULT_RIGHTS) : new JSONObject(raw);
            } catch (Exception e) {
                try { rights = new JSONObject(NodeDb.DEFAULT_RIGHTS); } catch (Exception e2) { rights = J.obj(); }
            }
            J.put(o, "roleRights", rights);
            J.put(o, "pubkey", c.isNull(7) ? null : c.getString(7));
            J.put(o, "createdAt", c.getString(8));
            JSONObject policy;
            try {
                String p = c.isNull(9) ? null : c.getString(9);
                policy = p == null || p.isEmpty() ? new JSONObject(NodeDb.DEFAULT_POLICY) : new JSONObject(p);
            } catch (Exception e) {
                try { policy = new JSONObject(NodeDb.DEFAULT_POLICY); } catch (Exception e2) { policy = J.obj(); }
            }
            J.put(o, "checkoutPolicy", policy);
            J.put(o, "guid", c.isNull(10) ? null : c.getString(10));
            return o;
        } catch (Exception e) {
            return null;
        }
    }

    JSONObject named(String table, Long id) {
        if (id == null) return null;
        try (Cursor c = n.q("SELECT id, name FROM " + table + " WHERE id=?", id)) {
            if (!c.moveToFirst()) return null;
            JSONObject o = J.obj();
            J.put(o, "id", c.getLong(0));
            J.put(o, "name", c.getString(1));
            return o;
        }
    }

    JSONObject statusObj(Long id) {
        if (id == null) return null;
        try (Cursor c = n.q("SELECT id, name, description, workspace_id, type, slug, color, bg FROM statuses WHERE id=?", id)) {
            if (!c.moveToFirst()) return null;
            JSONObject o = J.obj();
            J.put(o, "id", c.getLong(0));
            J.put(o, "name", c.getString(1));
            J.put(o, "description", c.isNull(2) ? null : c.getString(2));
            J.put(o, "workspaceId", c.getLong(3));
            J.put(o, "type", c.getString(4));
            J.put(o, "slug", c.getString(5));
            J.put(o, "color", c.getString(6));
            J.put(o, "bg", c.getString(7));
            return o;
        }
    }

    JSONObject storageObj(Long id) {
        if (id == null) return null;
        try (Cursor c = n.q("SELECT id, name, responsible_user_id, workspace_id, address FROM storages WHERE id=?", id)) {
            if (!c.moveToFirst()) return null;
            JSONObject o = J.obj();
            J.put(o, "id", c.getLong(0));
            J.put(o, "name", c.getString(1));
            J.put(o, "responsibleUserId", c.isNull(2) ? null : c.getLong(2));
            J.put(o, "workspaceId", c.getLong(3));
            J.put(o, "address", c.isNull(4) ? null : c.getString(4));
            return o;
        }
    }

    JSONObject workspaceJson(Long id) {
        if (id == null) return null;
        try (Cursor c = n.q("SELECT id, name, timezone, internal_id_prefix, comment, created_at, sync_url, guid FROM workspaces WHERE id=?", id)) {
            if (!c.moveToFirst()) return null;
            JSONObject o = J.obj();
            J.put(o, "id", c.getLong(0));
            J.put(o, "name", c.getString(1));
            J.put(o, "timezone", c.getString(2));
            J.put(o, "internalIdPrefix", c.getString(3));
            J.put(o, "comment", c.isNull(4) ? null : c.getString(4));
            J.put(o, "createdAt", c.getString(5));
            J.put(o, "syncUrl", c.isNull(6) ? null : c.getString(6));
            J.put(o, "guid", c.isNull(7) ? null : c.getString(7));
            return o;
        }
    }

    JSONArray photos(long itemId) {
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, item_id, url, is_title FROM item_photos WHERE item_id=?", itemId)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "itemId", c.getLong(1));
                J.put(o, "url", c.getString(2));
                J.put(o, "isTitle", c.getLong(3) != 0);
                a.put(o);
            }
        }
        return a;
    }

    JSONObject itemJson(Long id, boolean withHistory) {
        if (id == null) return null;
        JSONObject o;
        long categoryId, brandId, statusId, resp, site, storage, ws;
        boolean quantitative;
        try (Cursor c = n.q("SELECT id, internal_id, title, category_id, brand_id, status_id, responsible_user_id, building_site_id, storage_id, workspace_id, serial_number, cost, quantitative, quantity, unit, comment, qr_code, notify_date, created_at, due_at, guid, calibrated_until, min_quantity FROM items WHERE id=?", id)) {
            if (!c.moveToFirst()) return null;
            o = J.obj();
            J.put(o, "id", c.getLong(0));
            J.put(o, "internalId", c.getString(1));
            J.put(o, "title", c.getString(2));
            Long cat = c.isNull(3) ? null : c.getLong(3);
            Long br = c.isNull(4) ? null : c.getLong(4);
            Long st = c.isNull(5) ? null : c.getLong(5);
            Long rp = c.isNull(6) ? null : c.getLong(6);
            Long si = c.isNull(7) ? null : c.getLong(7);
            Long so = c.isNull(8) ? null : c.getLong(8);
            J.put(o, "categoryId", cat);
            J.put(o, "brandId", br);
            J.put(o, "statusId", st);
            J.put(o, "responsibleUserId", rp);
            J.put(o, "buildingSiteId", si);
            J.put(o, "storageId", so);
            J.put(o, "workspaceId", c.getLong(9));
            J.put(o, "serialNumber", c.isNull(10) ? null : c.getString(10));
            J.put(o, "cost", c.isNull(11) ? null : c.getDouble(11));
            quantitative = !c.isNull(12) && c.getLong(12) != 0;
            J.put(o, "quantitative", quantitative);
            J.put(o, "quantity", c.isNull(13) ? null : c.getDouble(13));
            J.put(o, "unit", c.isNull(14) ? null : c.getString(14));
            J.put(o, "comment", c.isNull(15) ? null : c.getString(15));
            J.put(o, "qrCode", c.isNull(16) ? null : c.getString(16));
            J.put(o, "notifyDate", c.isNull(17) ? null : c.getString(17));
            J.put(o, "createdAt", c.getString(18));
            J.put(o, "dueAt", c.isNull(19) ? null : c.getString(19));
            J.put(o, "guid", c.isNull(20) ? null : c.getString(20));
            J.put(o, "calibratedUntil", c.isNull(21) ? null : c.getString(21));
            J.put(o, "minQuantity", c.isNull(22) ? null : c.getDouble(22));
            J.put(o, "category", named("categories", cat));
            J.put(o, "brand", named("brands", br));
            J.put(o, "status", statusObj(st));
            JSONObject responsible = userPublic(rp);
            J.put(o, "responsible", responsible);
            J.put(o, "buildingSite", named("building_sites", si));
            J.put(o, "storage", storageObj(so));
            J.put(o, "photos", photos(c.getLong(0)));
            categoryId = cat == null ? 0 : cat;
            brandId = br == null ? 0 : br;
            statusId = st == null ? 0 : st;
            resp = rp == null ? 0 : rp;
            site = si == null ? 0 : si;
            storage = so == null ? 0 : so;
            ws = c.getLong(9);
        }
        attachHolders(id, o, quantitative);
        if (withHistory) {
            J.put(o, "history", historyForItem(id));
            J.put(o, "documents", docs(id));
            J.put(o, "comments", comments(id));
        }
        return o;
    }

    private void attachHolders(long id, JSONObject base, boolean quantitative) {
        JSONArray holders = J.arr();
        double issued = 0;
        try (Cursor c = n.q("SELECT id, user_id, quantity, due_at, created_at FROM item_holdings WHERE item_id=? AND returned_at IS NULL ORDER BY id DESC", id)) {
            while (c.moveToNext()) {
                long uid = c.getLong(1);
                double qv = c.getDouble(2);
                issued += qv;
                JSONObject h = J.obj();
                J.put(h, "id", c.getLong(0));
                J.put(h, "userId", uid);
                J.put(h, "quantity", qv);
                J.put(h, "dueAt", c.isNull(3) ? null : c.getString(3));
                J.put(h, "createdAt", c.getString(4));
                J.put(h, "user", userPublic(uid));
                holders.put(h);
            }
        }
        if (quantitative) {
            double stock = base.optDouble("quantity", 0);
            J.put(base, "stockQty", stock);
            J.put(base, "issuedQty", issued);
            J.put(base, "totalQty", stock + issued);
        } else if (!base.isNull("responsible") && base.optJSONObject("responsible") != null) {
            JSONObject h = J.obj();
            J.put(h, "id", 0);
            J.put(h, "userId", base.opt("responsibleUserId"));
            J.put(h, "quantity", 1);
            J.put(h, "dueAt", base.opt("dueAt"));
            J.put(h, "createdAt", base.opt("createdAt"));
            J.put(h, "user", base.opt("responsible"));
            J.put(h, "internalId", base.opt("internalId"));
            JSONArray shifted = J.arr();
            shifted.put(h);
            for (int i = 0; i < holders.length(); i++) shifted.put(holders.opt(i));
            holders = shifted;
        }
        long familyTotal = 0, familyStock = 0, familyIssued = 0;
        JSONArray members = J.arr();
        long ws = base.optLong("workspaceId", 0);
        String title = base.optString("title", "");
        if (ws > 0 && !title.isEmpty()) {
            try (Cursor c = n.q("SELECT id, internal_id, responsible_user_id, status_id FROM items WHERE workspace_id=? AND title=? ORDER BY id", ws, title)) {
                while (c.moveToNext()) {
                    long sid = c.getLong(0);
                    String vn = c.getString(1);
                    Long resp = c.isNull(2) ? null : c.getLong(2);
                    Long st = c.isNull(3) ? null : c.getLong(3);
                    familyTotal++;
                    if (resp != null) familyIssued++; else familyStock++;
                    JSONObject m = J.obj();
                    J.put(m, "id", sid);
                    J.put(m, "internalId", vn);
                    J.put(m, "responsibleUserId", resp);
                    J.put(m, "responsible", userPublic(resp));
                    J.put(m, "inStock", resp == null);
                    J.put(m, "status", statusObj(st));
                    members.put(m);
                    if (sid != id && resp != null) {
                        JSONObject h = J.obj();
                        J.put(h, "id", sid);
                        J.put(h, "userId", resp);
                        J.put(h, "quantity", 1);
                        J.put(h, "internalId", vn);
                        J.put(h, "user", userPublic(resp));
                        holders.put(h);
                    }
                }
            }
        }
        J.put(base, "holders", holders);
        JSONObject family = J.obj();
        J.put(family, "total", familyTotal);
        J.put(family, "inStock", familyStock);
        J.put(family, "issued", familyIssued);
        J.put(family, "members", members);
        J.put(base, "family", family);
        if (!quantitative) {
            J.put(base, "stockQty", familyStock);
            J.put(base, "issuedQty", familyIssued);
            J.put(base, "totalQty", familyTotal);
        }
    }

    JSONArray historyForItem(long itemId) {
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id FROM history_entries WHERE item_id=? ORDER BY id DESC LIMIT 50", itemId)) {
            while (c.moveToNext()) {
                JSONObject h = historyJson(c.getLong(0));
                if (h != null) a.put(h);
            }
        }
        return a;
    }

    JSONArray docs(long itemId) {
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, item_id, name, url FROM item_documents WHERE item_id=?", itemId)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "itemId", c.getLong(1));
                J.put(o, "name", c.getString(2));
                J.put(o, "url", c.getString(3));
                a.put(o);
            }
        }
        return a;
    }

    JSONArray comments(long itemId) {
        JSONArray a = J.arr();
        try (Cursor c = n.q("SELECT id, item_id, user_id, text, created_at FROM item_comments WHERE item_id=? AND active=1 ORDER BY id", itemId)) {
            while (c.moveToNext()) {
                JSONObject o = J.obj();
                J.put(o, "id", c.getLong(0));
                J.put(o, "itemId", c.getLong(1));
                J.put(o, "userId", c.getLong(2));
                J.put(o, "text", c.getString(3));
                J.put(o, "createdAt", c.getString(4));
                J.put(o, "user", userPublic(c.getLong(2)));
                a.put(o);
            }
        }
        return a;
    }

    JSONObject historyJson(long id) {
        try (Cursor c = n.q("SELECT id, workspace_id, item_id, type, actor_user_id, from_label, to_label, quantity_delta, comment, prev_hash, hash, signature, pubkey, created_at FROM history_entries WHERE id=?", id)) {
            if (!c.moveToFirst()) return null;
            JSONObject o = J.obj();
            long actor = c.getLong(4);
            Long itemId = c.isNull(2) ? null : c.getLong(2);
            J.put(o, "id", c.getLong(0));
            J.put(o, "workspaceId", c.getLong(1));
            J.put(o, "itemId", itemId);
            J.put(o, "type", c.getString(3));
            J.put(o, "actorUserId", actor);
            J.put(o, "fromLabel", c.isNull(5) ? null : c.getString(5));
            J.put(o, "toLabel", c.isNull(6) ? null : c.getString(6));
            J.put(o, "quantityDelta", c.isNull(7) ? null : c.getDouble(7));
            J.put(o, "comment", c.isNull(8) ? null : c.getString(8));
            J.put(o, "prevHash", c.isNull(9) ? null : c.getString(9));
            J.put(o, "hash", c.getString(10));
            J.put(o, "signature", c.isNull(11) ? null : c.getString(11));
            J.put(o, "pubkey", c.isNull(12) ? null : c.getString(12));
            J.put(o, "createdAt", c.getString(13));
            J.put(o, "actor", userPublic(actor));
            J.put(o, "item", itemId == null ? null : itemJson(itemId, false));
            return o;
        }
    }

    JSONObject transferJson(long id) {
        try (Cursor c = n.q("SELECT id, code, item_id, from_user_id, to_user_id, to_storage_id, building_site_id, workspace_id, quantity, status, photo_url, comment, no_confirmation, created_at, completed_at FROM transfers WHERE id=?", id)) {
            if (!c.moveToFirst()) return null;
            JSONObject o = J.obj();
            long itemId = c.getLong(2);
            long fromId = c.getLong(3);
            long toId = c.getLong(4);
            J.put(o, "id", c.getLong(0));
            J.put(o, "code", c.isNull(1) ? null : c.getString(1));
            J.put(o, "itemId", itemId);
            J.put(o, "fromUserId", fromId);
            J.put(o, "toUserId", toId);
            J.put(o, "toStorageId", c.isNull(5) ? null : c.getLong(5));
            J.put(o, "buildingSiteId", c.isNull(6) ? null : c.getLong(6));
            J.put(o, "workspaceId", c.getLong(7));
            J.put(o, "quantity", c.isNull(8) ? null : c.getDouble(8));
            J.put(o, "status", c.getString(9));
            J.put(o, "photoUrl", c.isNull(10) ? null : c.getString(10));
            J.put(o, "comment", c.isNull(11) ? null : c.getString(11));
            J.put(o, "noConfirmation", c.getLong(12) != 0);
            J.put(o, "createdAt", c.getString(13));
            J.put(o, "completedAt", c.isNull(14) ? null : c.getString(14));
            J.put(o, "item", itemJson(itemId, false));
            J.put(o, "fromUser", userPublic(fromId));
            J.put(o, "toUser", userPublic(toId));
            J.put(o, "toStorage", storageObj(c.isNull(5) ? null : c.getLong(5)));
            return o;
        }
    }
}
