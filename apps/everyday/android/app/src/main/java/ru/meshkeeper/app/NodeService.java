package ru.meshkeeper.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.ConnectivityManager;
import android.net.Network;
import android.os.Build;
import android.os.IBinder;
import android.util.Log;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.net.Inet4Address;
import java.net.NetworkInterface;
import java.util.Collections;

public class NodeService extends Service {
    public static final String EXTRA_RELAY = "relay";
    public static final String ACTION_ENABLE_BLE = "ru.meshkeeper.app.ENABLE_BLE";
    public static final String ACTION_SEND_BLE = "ru.meshkeeper.app.SEND_BLE";
    public static final String ACTION_DISABLE_BLE = "ru.meshkeeper.app.DISABLE_BLE";
    public static final String ACTION_BLE_STATUS = "ru.meshkeeper.app.BLE_STATUS";
    public static final String EXTRA_BLE_MESSAGE = "ble_message";
    public static final String EXTRA_BLE_ERROR = "ble_error";
    private static final String PREF_BLE_ENABLED = "ble_enabled";
    private static final String TAG = "MeshKeeperRustNode";
    private Thread nodeThread;
    private ConnectivityManager connectivityManager;
    private ConnectivityManager.NetworkCallback networkCallback;
    private BleMeshTransport bleTransport;

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String action = intent == null ? null : intent.getAction();
        boolean disableBle = ACTION_DISABLE_BLE.equals(action);
        boolean requestedBle = ACTION_ENABLE_BLE.equals(action) || ACTION_SEND_BLE.equals(action);
        boolean bleEnabled = !disableBle && (requestedBle || getSharedPreferences("meshkeeper", MODE_PRIVATE)
                .getBoolean(PREF_BLE_ENABLED, false));
        Notification n = notification();
        if (Build.VERSION.SDK_INT >= 34) {
            int type = ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC;
            if (bleEnabled) type |= ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE;
            startForeground(7, n, type);
        } else {
            startForeground(7, n);
        }
        watchNetworkChanges();
        if (disableBle) {
            getSharedPreferences("meshkeeper", MODE_PRIVATE).edit()
                    .putBoolean(PREF_BLE_ENABLED, false).apply();
            if (bleTransport != null) bleTransport.stop();
            bleTransport = null;
            publishBleStatus("BLE mesh выключен; очередь сохранена", false);
        } else if (requestedBle) {
            getSharedPreferences("meshkeeper", MODE_PRIVATE).edit()
                    .putBoolean(PREF_BLE_ENABLED, true).apply();
        }
        if (bleEnabled) {
            enableBle(ACTION_SEND_BLE.equals(action) || intent == null);
        }
        String relay = intent == null ? null : intent.getStringExtra(EXTRA_RELAY);
        if (relay == null) relay = getSharedPreferences("meshkeeper", MODE_PRIVATE).getString("relay", "");
        String workspaceScope = getSharedPreferences("meshkeeper", MODE_PRIVATE).getString("workspace_scope", "");
        String token;
        String capabilities;
        try {
            token = SecretStore.loadSyncToken(this);
            capabilities = SecretStore.loadSyncCapabilities(this);
        } catch (Exception error) {
            Log.e(TAG, "Не удалось расшифровать mesh-конфигурацию", error);
            stopSelf();
            return START_NOT_STICKY;
        }
        if (nodeThread == null || !nodeThread.isAlive()) {
            final String upstream = relay == null ? "" : relay;
            final String syncToken = token == null ? "" : token;
            final String syncCapabilities = capabilities == null ? "" : capabilities;
            final String scope = workspaceScope == null ? "" : workspaceScope;
            nodeThread = new Thread(() -> runNode(upstream, syncToken, scope, syncCapabilities), "meshkeeper-rust-node");
            nodeThread.start();
        }
        return START_STICKY;
    }

    private void runNode(String upstream, String token, String workspaceScope, String syncCapabilities) {
        if (!RustNode.isAvailable()) {
            Log.e(TAG, "libmeshkeeper_node.so отсутствует в APK");
            stopSelf();
            return;
        }
        try {
            File webRoot = new File(getFilesDir(), "www");
            extractAssets("www", webRoot);
            File db = new File(getNoBackupFilesDir(), "meshkeeper-rs.db");
            String nodeSigningKey = SecretStore.loadNodeSigningKey(this);
            if (nodeSigningKey.isEmpty()) {
                // Migration is crash-safe: Rust leaves the SQLite copy intact here.
                // It removes it only on startNode after comparing the sealed value.
                nodeSigningKey = RustNode.provisionNodeKey(db.getAbsolutePath());
                SecretStore.saveNodeSigningKey(this, nodeSigningKey);
            }
            String lan = lanIpv4();
            String advertised = lan.isEmpty() ? "" : "http://" + lan + ":" + RustNode.SYNC_PORT;
            RustNode.startNode(db.getAbsolutePath(), webRoot.getAbsolutePath(), upstream, token, workspaceScope,
                    syncCapabilities,
                    nodeSigningKey, advertised);
        } catch (Throwable error) {
            Log.e(TAG, "Rust-узел остановлен", error);
            stopSelf();
        }
    }

    private void extractAssets(String assetPath, File destination) throws Exception {
        String[] children = getAssets().list(assetPath);
        if (children != null && children.length > 0) {
            if (!destination.isDirectory() && !destination.mkdirs()) {
                throw new IllegalStateException("Не удалось создать " + destination);
            }
            for (String child : children) {
                extractAssets(assetPath + "/" + child, new File(destination, child));
            }
            return;
        }
        File parent = destination.getParentFile();
        if (parent != null && !parent.isDirectory() && !parent.mkdirs()) {
            throw new IllegalStateException("Не удалось создать " + parent);
        }
        try (InputStream input = getAssets().open(assetPath);
             FileOutputStream output = new FileOutputStream(destination)) {
            byte[] buffer = new byte[64 * 1024];
            int read;
            while ((read = input.read(buffer)) >= 0) output.write(buffer, 0, read);
        }
    }

    private static String lanIpv4() {
        try {
            for (NetworkInterface network : Collections.list(NetworkInterface.getNetworkInterfaces())) {
                if (!network.isUp() || network.isLoopback()) continue;
                for (java.net.InetAddress address : Collections.list(network.getInetAddresses())) {
                    if (address instanceof Inet4Address && !address.isLoopbackAddress()) {
                        return address.getHostAddress();
                    }
                }
            }
        } catch (Exception ignored) {}
        return "";
    }

    private void watchNetworkChanges() {
        if (networkCallback != null) return;
        connectivityManager = (ConnectivityManager) getSystemService(CONNECTIVITY_SERVICE);
        if (connectivityManager == null) return;
        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override public void onAvailable(Network network) { publishCurrentLanAddress(); }
            @Override public void onLost(Network network) { publishCurrentLanAddress(); }
        };
        try {
            connectivityManager.registerDefaultNetworkCallback(networkCallback);
        } catch (RuntimeException error) {
            Log.w(TAG, "Не удалось следить за сменой сети", error);
            networkCallback = null;
        }
    }

    private void publishCurrentLanAddress() {
        String lan = lanIpv4();
        if (lan.isEmpty() || !RustNode.isAvailable()) return;
        RustNode.updateAdvertiseUrl("http://" + lan + ":" + RustNode.SYNC_PORT);
    }

    private void enableBle(boolean sendPending) {
        try {
            if (bleTransport == null) {
                bleTransport = new BleMeshTransport(this, new BleMeshTransport.Listener() {
                    @Override public void onBundle(byte[] bundle) {
                        try {
                            BleBundleSpool.putIncoming(NodeService.this, bundle);
                            publishBleStatus("BLE-пакет принят и ожидает криптографической проверки", false);
                        } catch (Exception error) {
                            publishBleStatus("Не удалось сохранить BLE-пакет: " + error.getMessage(), true);
                        }
                    }

                    @Override public void onOutgoingDelivered() {
                        try {
                            BleBundleSpool.removeOutgoing(NodeService.this);
                        } catch (Exception error) {
                            publishBleStatus("Доставка завершена, но BLE spool не очищен: "
                                    + error.getMessage(), true);
                        }
                    }

                    @Override public void onStatus(String message, boolean error) {
                        publishBleStatus(message, error);
                    }
                });
            }
            bleTransport.enableReceiver();
            if (sendPending) {
                byte[] bundle = BleBundleSpool.peekOutgoing(this);
                if (bundle != null) bleTransport.send(bundle);
            }
        } catch (Exception error) {
            publishBleStatus("BLE foreground transport: " + error.getMessage(), true);
        }
    }

    private void publishBleStatus(String message, boolean error) {
        Log.println(error ? Log.WARN : Log.INFO, TAG, message);
        getSharedPreferences("meshkeeper", Context.MODE_PRIVATE).edit()
                .putString(EXTRA_BLE_MESSAGE, message)
                .putBoolean(EXTRA_BLE_ERROR, error)
                .apply();
        Intent update = new Intent(ACTION_BLE_STATUS)
                .setPackage(getPackageName())
                .putExtra(EXTRA_BLE_MESSAGE, message)
                .putExtra(EXTRA_BLE_ERROR, error);
        sendBroadcast(update);
    }

    @Override
    public void onDestroy() {
        if (bleTransport != null) bleTransport.stop();
        bleTransport = null;
        if (connectivityManager != null && networkCallback != null) {
            try {
                connectivityManager.unregisterNetworkCallback(networkCallback);
            } catch (RuntimeException ignored) {}
        }
        networkCallback = null;
        super.onDestroy();
    }

    private Notification notification() {
        String ch = "meshkeeper-node";
        NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (Build.VERSION.SDK_INT >= 26 && nm != null) {
            NotificationChannel c = new NotificationChannel(ch, "Узел MeshKeeper", NotificationManager.IMPORTANCE_LOW);
            c.setDescription("На телефоне работает локальный Rust-узел учёта");
            nm.createNotificationChannel(c);
        }
        Intent open = new Intent(this, MainActivity.class);
        PendingIntent pi = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder b;
        if (Build.VERSION.SDK_INT >= 26) b = new Notification.Builder(this, ch);
        else b = new Notification.Builder(this);
        return b.setContentTitle("MeshKeeper")
                .setContentText("Автономный Rust-узел работает локально")
                .setSmallIcon(android.R.drawable.stat_notify_sync)
                .setContentIntent(pi)
                .setOngoing(true)
                .build();
    }

    @Override
    public IBinder onBind(Intent intent) { return null; }
}
