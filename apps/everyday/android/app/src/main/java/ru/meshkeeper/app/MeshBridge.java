package ru.meshkeeper.app;

import android.webkit.JavascriptInterface;

import ru.meshkeeper.app.node.NodeRuntime;

public class MeshBridge {
    @JavascriptInterface
    public String lanOrigin() {
        return NodeRuntime.lanOrigin();
    }

    @JavascriptInterface
    public String localOrigin() {
        return NodeRuntime.localOrigin();
    }
}
