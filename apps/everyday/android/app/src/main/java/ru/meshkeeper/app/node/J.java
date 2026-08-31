package ru.meshkeeper.app.node;

import org.json.JSONArray;
import org.json.JSONObject;

final class J {
    private J() {}

    static JSONObject obj() { return new JSONObject(); }

    static JSONArray arr() { return new JSONArray(); }

    static JSONObject unwrap(Object raw) {
        if (raw instanceof JSONObject) {
            JSONObject o = (JSONObject) raw;
            if (o.has("json")) {
                Object inner = o.opt("json");
                if (inner instanceof JSONObject) {
                    JSONObject in = (JSONObject) inner;
                    if (in.has("json") && in.has("meta")) {
                        Object j2 = in.opt("json");
                        if (j2 instanceof JSONObject) return (JSONObject) j2;
                        JSONObject wrap = new JSONObject();
                        try { wrap.put("value", j2); } catch (Exception ignored) {}
                        return wrap;
                    }
                    return in;
                }
            }
            return o;
        }
        return new JSONObject();
    }

    static String str(JSONObject o, String k) {
        if (o == null || o.isNull(k)) return null;
        Object v = o.opt(k);
        if (v == null || v == JSONObject.NULL) return null;
        String s = String.valueOf(v).trim();
        return s.isEmpty() || "null".equals(s) ? null : s;
    }

    static Long lng(JSONObject o, String k) {
        if (o == null || o.isNull(k)) return null;
        Object v = o.opt(k);
        if (v == null || v == JSONObject.NULL) return null;
        if (v instanceof Number) return ((Number) v).longValue();
        try { return (long) Double.parseDouble(String.valueOf(v)); } catch (Exception e) { return null; }
    }

    static Double dbl(JSONObject o, String k) {
        if (o == null || o.isNull(k)) return null;
        Object v = o.opt(k);
        if (v == null || v == JSONObject.NULL) return null;
        if (v instanceof Number) return ((Number) v).doubleValue();
        try { return Double.parseDouble(String.valueOf(v)); } catch (Exception e) { return null; }
    }

    static Boolean bool(JSONObject o, String k) {
        if (o == null || o.isNull(k)) return null;
        Object v = o.opt(k);
        if (v instanceof Boolean) return (Boolean) v;
        if (v instanceof Number) return ((Number) v).intValue() != 0;
        String s = String.valueOf(v);
        if ("true".equalsIgnoreCase(s)) return true;
        if ("false".equalsIgnoreCase(s)) return false;
        return null;
    }

    static JSONArray arr(JSONObject o, String k) {
        return o == null ? null : o.optJSONArray(k);
    }

    static void put(JSONObject o, String k, Object v) {
        try {
            if (v == null) o.put(k, JSONObject.NULL);
            else o.put(k, v);
        } catch (Exception ignored) {}
    }

    static JSONObject ok(Object data) {
        JSONObject json = new JSONObject();
        JSONObject dataWrap = new JSONObject();
        JSONObject result = new JSONObject();
        JSONObject out = new JSONObject();
        try {
            dataWrap.put("json", data == null ? JSONObject.NULL : data);
            result.put("data", dataWrap);
            out.put("result", result);
        } catch (Exception ignored) {}
        return out;
    }

    static JSONObject err(String message, String code, int http) {
        int trpc = http == 401 ? -32001 : http == 404 ? -32004 : http == 409 ? -32009 : -32603;
        JSONObject data = new JSONObject();
        JSONObject json = new JSONObject();
        JSONObject error = new JSONObject();
        JSONObject out = new JSONObject();
        try {
            data.put("code", code);
            data.put("httpStatus", http);
            json.put("message", message);
            json.put("code", trpc);
            json.put("data", data);
            error.put("json", json);
            out.put("error", error);
        } catch (Exception ignored) {}
        return out;
    }
}
