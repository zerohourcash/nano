package ru.meshkeeper.app.node;

import android.content.res.AssetManager;
import android.util.Log;
import android.util.Base64;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

import fi.iki.elonen.NanoHTTPD;

final class LocalHttpServer extends NanoHTTPD {
    private static final String TAG = "MeshKeeperHttp";
    private final AssetManager assets;
    private final NodeDb db;
    private final TrpcDispatch trpc;
    private final Gossip gossip;
    private final Map<String, Long> sessions = new ConcurrentHashMap<>();
    private final SecureRandom secureRandom = new SecureRandom();

    LocalHttpServer(AssetManager assets, NodeDb db, TrpcDispatch trpc, Gossip gossip) {
        super("127.0.0.1", NodeRuntime.PORT);
        this.assets = assets;
        this.db = db;
        this.trpc = trpc;
        this.gossip = gossip;
    }

    @Override
    public Response serve(IHTTPSession session) {
        String uri = session.getUri();
        if (uri == null) uri = "/";
        Method method = session.getMethod();
        if (Method.OPTIONS.equals(method)) {
            return newFixedLengthResponse(Response.Status.FORBIDDEN, "text/plain", "cross-origin requests disabled");
        }
        try {
            if ("/health".equals(uri) || "/api/health".equals(uri)) {
                return json(gossip.hello());
            }
            if (uri.startsWith("/sync/")) {
                return newFixedLengthResponse(Response.Status.NOT_FOUND, "application/json", "{\"error\":\"sync disabled\"}");
            }
            if (uri.startsWith("/api/trpc")) {
                return handleTrpc(session, uri);
            }
            return serveAsset(uri);
        } catch (Exception e) {
            Log.e(TAG, "serve " + uri, e);
            Response r = newFixedLengthResponse(Response.Status.INTERNAL_ERROR, "application/json",
                    "{\"error\":\"" + e.getMessage() + "\"}");
            cors(r);
            return r;
        }
    }

    private Response handleTrpc(IHTTPSession session, String uri) throws Exception {
        String procedures = uri.substring("/api/trpc".length());
        if (procedures.startsWith("/")) procedures = procedures.substring(1);
        Map<String, List<String>> params = session.getParameters();
        String queryInput = first(params, "input");
        String batch = first(params, "batch");
        String body = Method.POST.equals(session.getMethod()) ? readBody(session) : null;
        List<Call> calls = parseCalls(procedures, queryInput, body);
        boolean batched = calls.size() > 1 || "1".equals(batch);
        Long uid = userFrom(session);
        String currentToken = sessionToken(session);
        JSONArray out = new JSONArray();
        Long setUser = null;
        for (Call call : calls) {
            try {
                Object data = trpc.dispatch(call.name, call.input, uid);
                out.put(J.ok(data));
                if ("auth.login".equals(call.name) || "auth.register".equals(call.name) || "auth.joinRegister".equals(call.name)) {
                    if (data instanceof JSONObject) setUser = ((JSONObject) data).optLong("id", 0);
                }
                if ("auth.logout".equals(call.name)) {
                    if (currentToken != null) sessions.remove(currentToken);
                    setUser = 0L;
                }
            } catch (ApiEx e) {
                out.put(J.err(e.getMessage(), e.code, e.http));
            } catch (Exception e) {
                Log.e(TAG, call.name, e);
                out.put(J.err(e.getMessage() == null ? "ошибка" : e.getMessage(), "BAD_REQUEST", 400));
            }
        }
        String payload = batched || out.length() != 1 ? out.toString() : out.getJSONObject(0).toString();
        Response r = newFixedLengthResponse(Response.Status.OK, "application/json; charset=utf-8", payload);
        cors(r);
        if (setUser != null) {
            if (setUser == 0) {
                r.addHeader("Set-Cookie", "mk_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0");
            } else {
                byte[] raw = new byte[32];
                secureRandom.nextBytes(raw);
                String token = Base64.encodeToString(raw, Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
                sessions.put(token, setUser);
                r.addHeader("Set-Cookie", "mk_session=" + token + "; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000");
            }
        }
        return r;
    }

    private static class Call {
        final String name;
        final JSONObject input;
        Call(String n, JSONObject i) { name = n; input = i; }
    }

    private List<Call> parseCalls(String procedures, String queryInput, String body) {
        List<String> names = new ArrayList<>();
        if (procedures != null) {
            for (String p : procedures.split(",")) {
                String n = p.trim();
                if (n.startsWith("/")) n = n.substring(1);
                if (!n.isEmpty()) names.add(n);
            }
        }
        Object raw = null;
        try {
            if (body != null && !body.isEmpty()) raw = parseRaw(body);
            else if (queryInput != null && !queryInput.isEmpty()) raw = parseRaw(queryInput);
        } catch (Exception ignored) {}
        List<Call> out = new ArrayList<>();
        if (raw instanceof JSONObject) {
            JSONObject map = (JSONObject) raw;
            if (map.has("0") || looksIndexed(map)) {
                for (int i = 0; i < names.size(); i++) {
                    Object inp = map.opt(String.valueOf(i));
                    out.add(new Call(names.get(i), J.unwrap(inp)));
                }
                return out;
            }
            JSONObject un = J.unwrap(map);
            if (names.size() == 1) {
                out.add(new Call(names.get(0), un));
                return out;
            }
            for (String n : names) out.add(new Call(n, un));
            return out;
        }
        for (String n : names) out.add(new Call(n, J.obj()));
        return out;
    }

    private static boolean looksIndexed(JSONObject map) {
        java.util.Iterator<String> it = map.keys();
        while (it.hasNext()) {
            try { Integer.parseInt(it.next()); return true; } catch (Exception ignored) {}
        }
        return false;
    }

    private static Object parseRaw(String s) throws Exception {
        String t = s.trim();
        if (t.startsWith("%")) t = URLDecoder.decode(t, "UTF-8");
        if (t.startsWith("[")) return new JSONArray(t);
        return new JSONObject(t);
    }

    private Long userFrom(IHTTPSession session) {
        String token = sessionToken(session);
        return token == null ? null : sessions.get(token);
    }

    private String sessionToken(IHTTPSession session) {
        String cookie = session.getHeaders().get("cookie");
        if (cookie != null) {
            for (String part : cookie.split(";")) {
                String p = part.trim();
                if (p.startsWith("mk_session=")) return p.substring("mk_session=".length()).trim();
            }
        }
        return null;
    }

    private String readBody(IHTTPSession session) {
        Map<String, String> files = new HashMap<>();
        try {
            session.parseBody(files);
        } catch (Exception e) {
            return "";
        }
        String post = files.get("postData");
        return post == null ? "" : post;
    }

    private Response serveAsset(String uri) {
        String path = uri;
        int q = path.indexOf('?');
        if (q >= 0) path = path.substring(0, q);
        if (path.startsWith("/")) path = path.substring(1);
        if (path.isEmpty()) path = "index.html";
        InputStream in = openWww(path);
        if (in == null) {
            if (!path.contains(".")) in = openWww("index.html");
            path = in == null ? path : "index.html";
        }
        if (in == null) {
            Response r = newFixedLengthResponse(Response.Status.NOT_FOUND, "text/plain", "not found");
            cors(r);
            return r;
        }
        try {
            ByteArrayOutputStream bos = new ByteArrayOutputStream();
            byte[] buf = new byte[8192];
            int n;
            while ((n = in.read(buf)) > 0) bos.write(buf, 0, n);
            in.close();
            byte[] data = bos.toByteArray();
            Response r = newFixedLengthResponse(Response.Status.OK, mime(path), new ByteArrayInputStream(data), data.length);
            cors(r);
            return r;
        } catch (Exception e) {
            return newFixedLengthResponse(Response.Status.INTERNAL_ERROR, "text/plain", "io");
        }
    }

    private InputStream openWww(String path) {
        try {
            return assets.open("www/" + path);
        } catch (Exception e) {
            return null;
        }
    }

    private static String mime(String path) {
        String p = path.toLowerCase();
        if (p.endsWith(".js")) return "application/javascript; charset=utf-8";
        if (p.endsWith(".css")) return "text/css; charset=utf-8";
        if (p.endsWith(".html")) return "text/html; charset=utf-8";
        if (p.endsWith(".json") || p.endsWith(".webmanifest")) return "application/json";
        if (p.endsWith(".png")) return "image/png";
        if (p.endsWith(".jpg") || p.endsWith(".jpeg")) return "image/jpeg";
        if (p.endsWith(".svg")) return "image/svg+xml";
        if (p.endsWith(".woff2")) return "font/woff2";
        if (p.endsWith(".woff")) return "font/woff";
        return "application/octet-stream";
    }

    private Response json(JSONObject o) {
        Response r = newFixedLengthResponse(Response.Status.OK, "application/json; charset=utf-8", o.toString());
        cors(r);
        return r;
    }

    private static void cors(Response r) {
        r.addHeader("X-Content-Type-Options", "nosniff");
        r.addHeader("Content-Security-Policy", "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'");
    }

    private static String first(Map<String, List<String>> params, String k) {
        if (params == null) return null;
        List<String> v = params.get(k);
        return v == null || v.isEmpty() ? null : v.get(0);
    }
}
