package ru.meshkeeper.app.node;

final class ApiEx extends Exception {
    final String code;
    final int http;

    ApiEx(String code, int http, String message) {
        super(message);
        this.code = code;
        this.http = http;
    }

    static ApiEx unauth(String m) { return new ApiEx("UNAUTHORIZED", 401, m); }
    static ApiEx notFound(String m) { return new ApiEx("NOT_FOUND", 404, m); }
    static ApiEx bad(String m) { return new ApiEx("BAD_REQUEST", 400, m); }
    static ApiEx conflict(String m) { return new ApiEx("CONFLICT", 409, m); }
    static ApiEx forbidden() { return new ApiEx("FORBIDDEN", 403, "Недостаточно прав для этого действия"); }
}
