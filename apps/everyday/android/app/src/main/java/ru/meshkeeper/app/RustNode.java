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
            String workspaceScope,
            String syncCapabilities,
            String nodeSigningKey,
            String advertiseUrl
    );

    /** Returns the existing/new node seed so Java can seal it before Rust removes SQLite plaintext. */
    public static native String provisionNodeKey(String dbPath);

    /** Updates the LAN endpoint announced by the already running Rust node. */
    public static native void updateAdvertiseUrl(String advertiseUrl);

    /** Opaque encrypted bundle/CAS framing for BLE GATT, LoRa or serial links. */
    public static native byte[][] fragmentTransport(byte[] payload, int mtu, int kind);

    /** Bounded structural and chunk-tag verification before a radio inbox retains a frame. */
    public static native void validateTransportFrame(byte[] frame);

    /** Returns inclusive missing sequence ranges as JSON, for example [[2,4],[9,9]]. */
    public static native String missingTransportRanges(byte[][] receivedFrames);

    /** Reassembles and verifies all frames; throws while frames are missing or invalid. */
    public static native byte[] assembleTransport(byte[][] receivedFrames);

    public static String localOrigin() { return "http://localhost:" + UI_PORT; }
}
