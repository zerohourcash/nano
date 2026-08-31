package ru.meshkeeper.app.node;

import android.content.Context;
import android.database.Cursor;
import android.database.sqlite.SQLiteDatabase;
import android.database.sqlite.SQLiteStatement;

import org.json.JSONObject;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import javax.crypto.SecretKeyFactory;
import javax.crypto.spec.PBEKeySpec;
import java.security.SecureRandom;
import java.util.UUID;

final class NodeDb {
    static final String OWNER_RIGHTS = "{\"viewItems\":true,\"createItems\":true,\"editItems\":true,\"deleteItems\":true,\"transferItems\":true,\"acceptTransfers\":true,\"writeOff\":true,\"replenish\":true,\"inventory\":true,\"viewHistory\":true,\"viewReports\":true,\"manageUsers\":true,\"manageWorkspaces\":true,\"manageStorages\":true,\"manageSites\":true,\"manageDictionaries\":true,\"reportFaults\":true,\"requestChanges\":true}";
    static final String DEFAULT_RIGHTS = "{\"viewItems\":true,\"createItems\":true,\"editItems\":true,\"deleteItems\":false,\"transferItems\":true,\"acceptTransfers\":true,\"writeOff\":false,\"replenish\":true,\"inventory\":true,\"viewHistory\":true,\"viewReports\":true,\"manageUsers\":false,\"manageWorkspaces\":false,\"manageStorages\":false,\"manageSites\":false,\"manageDictionaries\":false,\"reportFaults\":true,\"requestChanges\":true}";
    static final String DEFAULT_POLICY = "{\"allowedCategoryIds\":null,\"maxHours\":null,\"requireApproval\":false,\"allowNoDueDate\":true}";

    final SQLiteDatabase db;
    private final SecureRandom rnd = new SecureRandom();

    NodeDb(Context ctx) {
        db = ctx.openOrCreateDatabase("meshkeeper.db", Context.MODE_PRIVATE, null);
        db.enableWriteAheadLogging();
        db.execSQL("PRAGMA foreign_keys=ON");
        init();
        migrate();
        fillGuids();
        ensureNode();
    }

    synchronized void close() {
        if (db.isOpen()) db.close();
    }

    static String now() {
        java.text.SimpleDateFormat f = new java.text.SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss.SSS'Z'", java.util.Locale.US);
        f.setTimeZone(java.util.TimeZone.getTimeZone("UTC"));
        return f.format(new java.util.Date());
    }

    static String guid() {
        return UUID.randomUUID().toString().replace("-", "");
    }

    static String digits(String phone) {
        if (phone == null) return "";
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < phone.length(); i++) {
            char c = phone.charAt(i);
            if (c >= '0' && c <= '9') b.append(c);
        }
        String d = b.toString();
        if (d.startsWith("8") && d.length() == 11) d = "7" + d.substring(1);
        return d;
    }

    String hashPassword(String password) {
        byte[] saltB = new byte[16];
        rnd.nextBytes(saltB);
        try {
            int iterations = 210000;
            PBEKeySpec spec = new PBEKeySpec(password.toCharArray(), saltB, iterations, 256);
            byte[] digest = SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256").generateSecret(spec).getEncoded();
            spec.clearPassword();
            return "$pbkdf2$" + iterations + "$" + hex(saltB) + "$" + hex(digest);
        } catch (Exception e) {
            throw new IllegalStateException("Password hashing unavailable", e);
        }
    }

    boolean verifyPassword(String password, String stored) {
        if (stored == null) return false;
        if (stored.startsWith("$pbkdf2$")) {
            try {
                String[] parts = stored.split("\\$");
                int iterations = Integer.parseInt(parts[2]);
                byte[] salt = unhex(parts[3]);
                byte[] expected = unhex(parts[4]);
                PBEKeySpec spec = new PBEKeySpec(password.toCharArray(), salt, iterations, expected.length * 8);
                byte[] actual = SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256").generateSecret(spec).getEncoded();
                spec.clearPassword();
                return MessageDigest.isEqual(actual, expected);
            } catch (Exception e) { return false; }
        }
        int i = stored.indexOf('$');
        if (i < 0) return false;
        String salt = stored.substring(0, i);
        String digest = stored.substring(i + 1);
        return sha256Hex(salt + ":" + password).equals(digest);
    }

    static byte[] unhex(String value) {
        if ((value.length() & 1) != 0) throw new IllegalArgumentException("bad hex");
        byte[] out = new byte[value.length() / 2];
        for (int i = 0; i < out.length; i++) out[i] = (byte) Integer.parseInt(value.substring(i * 2, i * 2 + 2), 16);
        return out;
    }

    static String sha256Hex(String s) {
        try {
            MessageDigest md = MessageDigest.getInstance("SHA-256");
            return hex(md.digest(s.getBytes(StandardCharsets.UTF_8)));
        } catch (Exception e) {
            return "";
        }
    }

    static String hex(byte[] b) {
        StringBuilder sb = new StringBuilder(b.length * 2);
        for (byte x : b) sb.append(String.format("%02x", x));
        return sb.toString();
    }

    synchronized Cursor q(String sql, Object... args) {
        return db.rawQuery(sql, strArgs(args));
    }

    synchronized long long1(String sql, Object... args) {
        try (Cursor c = q(sql, args)) {
            if (c.moveToFirst() && !c.isNull(0)) return c.getLong(0);
            return 0;
        }
    }

    synchronized Long longN(String sql, Object... args) {
        try (Cursor c = q(sql, args)) {
            if (c.moveToFirst() && !c.isNull(0)) return c.getLong(0);
            return null;
        }
    }

    synchronized String str1(String sql, Object... args) {
        try (Cursor c = q(sql, args)) {
            if (c.moveToFirst() && !c.isNull(0)) return c.getString(0);
            return null;
        }
    }

    synchronized void exec(String sql, Object... args) {
        if (args == null || args.length == 0) {
            db.execSQL(sql);
            return;
        }
        db.execSQL(sql, objArgs(args));
    }

    synchronized long insert(String sql, Object... args) {
        SQLiteStatement st = db.compileStatement(sql);
        try {
            bind(st, args);
            return st.executeInsert();
        } finally {
            st.close();
        }
    }

    synchronized String kvGet(String k) {
        return str1("SELECT v FROM kv WHERE k=?", k);
    }

    synchronized void kvSet(String k, String v) {
        exec("INSERT OR REPLACE INTO kv(k,v) VALUES(?,?)", k, v);
    }

    synchronized Long findUserPhone(String phone) {
        String want = digits(phone);
        try (Cursor c = q("SELECT id, phone FROM users")) {
            while (c.moveToNext()) {
                if (digits(c.getString(1)).equals(want)) return c.getLong(0);
            }
        }
        return null;
    }

    synchronized long wsFallback(Long uid) {
        if (uid != null) {
            Long w = longN("SELECT workspace_id FROM user_workspaces WHERE user_id=? ORDER BY id DESC LIMIT 1", uid);
            if (w != null) return w;
        }
        Long w = longN("SELECT id FROM workspaces ORDER BY id LIMIT 1");
        return w == null ? 1 : w;
    }

    synchronized boolean userCan(long uid, String key) {
        String raw = str1("SELECT role_rights FROM users WHERE id=?", uid);
        JSONObject rights;
        try {
            rights = raw == null || raw.isEmpty() ? new JSONObject(DEFAULT_RIGHTS) : new JSONObject(raw);
        } catch (Exception e) {
            try { rights = new JSONObject(DEFAULT_RIGHTS); } catch (Exception e2) { return false; }
        }
        return rights.optBoolean(key, false);
    }

    synchronized void requireUser(Long uid) throws ApiEx {
        if (uid == null) throw ApiEx.unauth("Войдите в систему");
        String status = str1("SELECT status FROM users WHERE id=?", uid);
        if (status == null) throw ApiEx.unauth("Пользователь не найден");
        if ("disabled".equals(status)) throw ApiEx.unauth("Аккаунт заблокирован");
    }

    synchronized void requireCan(long uid, String key) throws ApiEx {
        if (!userCan(uid, key)) throw ApiEx.forbidden();
    }

    synchronized void requireMember(long uid, long workspaceId) throws ApiEx {
        if (long1("SELECT COUNT(*) FROM user_workspaces WHERE user_id=? AND workspace_id=?", uid, workspaceId) == 0) {
            throw ApiEx.forbidden();
        }
    }

    synchronized String appendLedger(long ws, long actor, Long itemId, String type, String from, String to, Double qty, String comment) {
        String prev = str1("SELECT hash FROM history_entries WHERE workspace_id=? ORDER BY id DESC LIMIT 1", ws);
        String ts = now();
        String payload = ws + "|" + actor + "|" + itemId + "|" + type + "|" + from + "|" + to + "|" + qty + "|" + comment + "|" + prev + "|" + ts;
        String hash = sha256Hex(payload);
        insert("INSERT INTO history_entries (workspace_id,item_id,type,actor_user_id,from_label,to_label,quantity_delta,comment,prev_hash,hash,created_at,guid) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                ws, itemId, type, actor, from, to, qty, comment, prev, hash, ts, guid());
        return hash;
    }

    synchronized void seedWorkspaceStatuses(long ws) {
        exec("INSERT INTO statuses (name, workspace_id, type, slug, color, bg) VALUES ('В работе',?,'status','in-work','#2E9E5B','#C8FCD2')", ws);
        exec("INSERT INTO statuses (name, workspace_id, type, slug, color, bg) VALUES ('В ремонте',?,'status','in-repair','#A87C0F','#FBFCC8')", ws);
        exec("INSERT INTO statuses (name, workspace_id, type, slug, color, bg) VALUES ('На складе',?,'status','in-stock','#5E629B','#EDEDF7')", ws);
        exec("INSERT INTO statuses (name, workspace_id, type, slug, color, bg) VALUES ('Списан',?,'status','written-off','#D64545','#FAD8D1')", ws);
        exec("INSERT INTO statuses (name, workspace_id, type, slug, color, bg) VALUES ('На проверке',?,'status','needs-check','#A87C0F','#FBFCC8')", ws);
    }

    private void ensureNode() {
        if (kvGet("node_id") == null) {
            kvSet("node_id", guid());
            kvSet("node_pk", guid());
            kvSet("node_sk", guid());
            kvSet("node_name", android.os.Build.MODEL == null ? "MeshKeeper" : "Телефон " + android.os.Build.MODEL);
        }
    }

    private void fillGuids() {
        for (String t : new String[]{"workspaces", "users", "items", "history_entries"}) {
            exec("UPDATE " + t + " SET guid=lower(hex(randomblob(16))) WHERE guid IS NULL OR guid=''");
        }
    }

    private static String[] strArgs(Object[] args) {
        if (args == null) return null;
        String[] out = new String[args.length];
        for (int i = 0; i < args.length; i++) out[i] = args[i] == null ? null : String.valueOf(args[i]);
        return out;
    }

    private static Object[] objArgs(Object[] args) {
        Object[] out = new Object[args.length];
        for (int i = 0; i < args.length; i++) out[i] = args[i] == null ? null : args[i];
        return out;
    }

    private static void bind(SQLiteStatement st, Object[] args) {
        for (int i = 0; i < args.length; i++) {
            Object v = args[i];
            int n = i + 1;
            if (v == null) st.bindNull(n);
            else if (v instanceof Long) st.bindLong(n, (Long) v);
            else if (v instanceof Integer) st.bindLong(n, (Integer) v);
            else if (v instanceof Double) st.bindDouble(n, (Double) v);
            else if (v instanceof Float) st.bindDouble(n, (Float) v);
            else if (v instanceof Boolean) st.bindLong(n, (Boolean) v ? 1 : 0);
            else st.bindString(n, String.valueOf(v));
        }
    }

    private void init() {
        db.execSQL("CREATE TABLE IF NOT EXISTS workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, timezone TEXT NOT NULL DEFAULT 'Europe/Moscow', internal_id_prefix TEXT NOT NULL DEFAULT 'ВН-', comment TEXT, created_at TEXT NOT NULL, guid TEXT, sync_url TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY AUTOINCREMENT, full_name TEXT NOT NULL, position TEXT, phone TEXT NOT NULL UNIQUE, avatar_url TEXT, status TEXT NOT NULL DEFAULT 'active', password_hash TEXT, role_rights TEXT, pubkey TEXT, privkey TEXT, created_at TEXT NOT NULL, guid TEXT, checkout_policy TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS user_workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, workspace_id INTEGER NOT NULL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS storages (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, responsible_user_id INTEGER, workspace_id INTEGER NOT NULL, address TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS building_sites (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, responsible_user_id INTEGER, workspace_id INTEGER NOT NULL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS categories (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, description TEXT, workspace_id INTEGER NOT NULL, type TEXT NOT NULL DEFAULT 'category')");
        db.execSQL("CREATE TABLE IF NOT EXISTS brands (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, description TEXT, workspace_id INTEGER NOT NULL, type TEXT NOT NULL DEFAULT 'brand')");
        db.execSQL("CREATE TABLE IF NOT EXISTS statuses (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, description TEXT, workspace_id INTEGER NOT NULL, type TEXT NOT NULL DEFAULT 'status', slug TEXT NOT NULL DEFAULT 'in-stock', color TEXT NOT NULL DEFAULT '#5E629B', bg TEXT NOT NULL DEFAULT '#EDEDF7')");
        db.execSQL("CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY AUTOINCREMENT, internal_id TEXT NOT NULL, title TEXT NOT NULL, category_id INTEGER, brand_id INTEGER, status_id INTEGER, responsible_user_id INTEGER, building_site_id INTEGER, storage_id INTEGER, workspace_id INTEGER NOT NULL, serial_number TEXT, cost REAL, quantitative INTEGER NOT NULL DEFAULT 0, quantity REAL, unit TEXT, comment TEXT, qr_code TEXT, notify_date TEXT, due_at TEXT, created_at TEXT NOT NULL, guid TEXT, calibrated_until TEXT, min_quantity REAL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS item_photos (id INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER NOT NULL, url TEXT NOT NULL, is_title INTEGER NOT NULL DEFAULT 0)");
        db.execSQL("CREATE TABLE IF NOT EXISTS item_documents (id INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER NOT NULL, name TEXT NOT NULL, url TEXT NOT NULL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS item_comments (id INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER NOT NULL, user_id INTEGER NOT NULL, text TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS transfers (id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT, item_id INTEGER NOT NULL, from_user_id INTEGER NOT NULL, to_user_id INTEGER NOT NULL, to_storage_id INTEGER, building_site_id INTEGER, workspace_id INTEGER NOT NULL, quantity REAL, status TEXT NOT NULL DEFAULT 'pending', photo_url TEXT, comment TEXT, no_confirmation INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL, completed_at TEXT, needs_admin INTEGER NOT NULL DEFAULT 0)");
        db.execSQL("CREATE TABLE IF NOT EXISTS history_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id INTEGER NOT NULL, item_id INTEGER, type TEXT NOT NULL, actor_user_id INTEGER NOT NULL, from_label TEXT, to_label TEXT, quantity_delta REAL, comment TEXT, prev_hash TEXT, hash TEXT NOT NULL, signature TEXT, pubkey TEXT, created_at TEXT NOT NULL, guid TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS inventory_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, number TEXT NOT NULL, workspace_id INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'in_progress', started_by INTEGER NOT NULL, created_at TEXT NOT NULL, completed_at TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS inventory_results (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id INTEGER NOT NULL, item_id INTEGER NOT NULL, expected_qty REAL, actual_qty REAL, checked INTEGER NOT NULL DEFAULT 0)");
        db.execSQL("CREATE TABLE IF NOT EXISTS notifications (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER NOT NULL, item_id INTEGER, type TEXT NOT NULL, title TEXT, text TEXT NOT NULL, read INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS invites (id INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id INTEGER NOT NULL, token TEXT NOT NULL UNIQUE, role TEXT NOT NULL DEFAULT 'member', created_by INTEGER, expires_at TEXT, max_uses INTEGER NOT NULL DEFAULT 20, used_count INTEGER NOT NULL DEFAULT 0, revoked INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS faults (id INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER NOT NULL, workspace_id INTEGER NOT NULL, author_id INTEGER NOT NULL, severity TEXT NOT NULL DEFAULT 'medium', description TEXT NOT NULL, photo_url TEXT, status TEXT NOT NULL DEFAULT 'open', resolution TEXT, resolver_id INTEGER, created_at TEXT NOT NULL, resolved_at TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS change_requests (id INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER NOT NULL, workspace_id INTEGER NOT NULL, author_id INTEGER NOT NULL, payload TEXT NOT NULL, comment TEXT, status TEXT NOT NULL DEFAULT 'pending', reason TEXT, decided_by INTEGER, created_at TEXT NOT NULL, decided_at TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS chat_messages (id INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id INTEGER NOT NULL, user_id INTEGER NOT NULL, text TEXT NOT NULL, created_at TEXT NOT NULL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL)");
        db.execSQL("CREATE TABLE IF NOT EXISTS peers (id INTEGER PRIMARY KEY AUTOINCREMENT, node_id TEXT, url TEXT NOT NULL UNIQUE, name TEXT, last_seen TEXT, last_sync TEXT, last_error TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS item_holdings (id INTEGER PRIMARY KEY AUTOINCREMENT, item_id INTEGER NOT NULL, user_id INTEGER NOT NULL, quantity REAL NOT NULL DEFAULT 1, due_at TEXT, comment TEXT, photo_url TEXT, created_at TEXT NOT NULL, returned_at TEXT)");
        db.execSQL("CREATE TABLE IF NOT EXISTS conflicts (id INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id INTEGER, item_id INTEGER, item_guid TEXT, status TEXT NOT NULL DEFAULT 'open', description TEXT NOT NULL, left_label TEXT, right_label TEXT, created_at TEXT NOT NULL, resolved_at TEXT, resolver_id INTEGER)");
        db.execSQL("CREATE UNIQUE INDEX IF NOT EXISTS hist_hash_uq ON history_entries(hash)");
    }

    private void migrate() {
        String[] alters = {
                "ALTER TABLE items ADD COLUMN due_at TEXT",
                "ALTER TABLE items ADD COLUMN guid TEXT",
                "ALTER TABLE items ADD COLUMN calibrated_until TEXT",
                "ALTER TABLE items ADD COLUMN min_quantity REAL",
                "ALTER TABLE users ADD COLUMN guid TEXT",
                "ALTER TABLE users ADD COLUMN checkout_policy TEXT",
                "ALTER TABLE workspaces ADD COLUMN guid TEXT",
                "ALTER TABLE workspaces ADD COLUMN sync_url TEXT",
                "ALTER TABLE history_entries ADD COLUMN guid TEXT",
                "ALTER TABLE transfers ADD COLUMN needs_admin INTEGER NOT NULL DEFAULT 0",
                "ALTER TABLE transfers ADD COLUMN photo_url TEXT"
        };
        for (String a : alters) {
            try { db.execSQL(a); } catch (Exception ignored) {}
        }
    }
}
