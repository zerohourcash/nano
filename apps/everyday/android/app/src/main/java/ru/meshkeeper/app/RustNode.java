package ru.meshkeeper.app;

/** Единственный bridge к production Rust-узлу; бизнес-логики здесь нет. */
public final class RustNode {
    public static final int UI_PORT = 8765;
    public static final int SYNC_PORT = 8766;
    private static final boolean AVAILABLE;

    static {
        boolean loaded;
        try {
            System.loadLibrary("meshkeeper_node");
            loaded = true;
        } catch (UnsatisfiedLinkError error) {
            loaded = false;
        }
        AVAILABLE = loaded;
    }

    private RustNode() {}

    public static boolean isAvailable() { return AVAILABLE; }

    public static native int startNode(
            String dbPath,
            String webRoot,
            String upstream,
            String syncToken,
            String advertiseUrl
    );

    public static String localOrigin() { return "http://localhost:" + UI_PORT; }
}
