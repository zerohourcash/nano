package ru.meshkeeper.app.node;

import android.content.Context;
import android.util.Log;

public final class NodeRuntime {
    public static final int PORT = 8765;
    private static final String TAG = "MeshKeeperNode";
    private static NodeRuntime inst;

    final NodeDb db;
    final JsonShapes json;
    final Gossip gossip;
    final TrpcDispatch trpc;
    final LocalHttpServer http;
    final String relay;

    private NodeRuntime(Context ctx, String relay) throws Exception {
        this.relay = relay == null ? "" : relay.trim().replaceAll("/+$", "");
        db = new NodeDb(ctx.getApplicationContext());
        json = new JsonShapes(db);
        gossip = new Gossip(db, json);
        trpc = new TrpcDispatch(db, json, gossip);
        http = new LocalHttpServer(ctx.getAssets(), db, trpc, gossip);
        http.start();
        // Legacy gossip is intentionally disabled until the authenticated sync-v2 protocol exists.
        Log.i(TAG, "node on " + gossip.lanOrigin);
    }

    public static synchronized NodeRuntime start(Context ctx, String relay) throws Exception {
        if (inst != null) {
            return inst;
        }
        inst = new NodeRuntime(ctx.getApplicationContext(), relay);
        return inst;
    }

    public static synchronized NodeRuntime get() { return inst; }

    public static String lanOrigin() {
        return inst == null ? "http://127.0.0.1:" + PORT : inst.gossip.lanOrigin();
    }

    public static String localOrigin() {
        return "http://localhost:" + PORT;
    }

    public static String lanIpv4() {
        return Gossip.lanIpv4();
    }
}
