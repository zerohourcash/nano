package ru.meshkeeper.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
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
    private static final String TAG = "MeshKeeperRustNode";
    private Thread nodeThread;

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        Notification n = notification();
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(7, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
        } else {
            startForeground(7, n);
        }
        String relay = intent == null ? null : intent.getStringExtra(EXTRA_RELAY);
        if (relay == null) relay = getSharedPreferences("meshkeeper", MODE_PRIVATE).getString("relay", "");
        String token;
        try {
            token = SecretStore.loadSyncToken(this);
        } catch (Exception error) {
            Log.e(TAG, "Не удалось расшифровать mesh-токен", error);
            stopSelf();
            return START_NOT_STICKY;
        }
        if (nodeThread == null || !nodeThread.isAlive()) {
            final String upstream = relay == null ? "" : relay;
            final String syncToken = token == null ? "" : token;
            nodeThread = new Thread(() -> runNode(upstream, syncToken), "meshkeeper-rust-node");
            nodeThread.start();
        }
        return START_STICKY;
    }

    private void runNode(String upstream, String token) {
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
            RustNode.startNode(db.getAbsolutePath(), webRoot.getAbsolutePath(), upstream, token,
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
