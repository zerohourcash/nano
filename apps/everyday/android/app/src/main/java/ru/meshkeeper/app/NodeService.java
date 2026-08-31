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

import ru.meshkeeper.app.node.NodeRuntime;

public class NodeService extends Service {
    public static final String EXTRA_RELAY = "relay";

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String relay = intent == null ? "" : intent.getStringExtra(EXTRA_RELAY);
        try {
            NodeRuntime.start(this, relay);
        } catch (Exception e) {
            stopSelf();
            return START_NOT_STICKY;
        }
        Notification n = notification();
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(7, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
        } else {
            startForeground(7, n);
        }
        return START_STICKY;
    }

    private Notification notification() {
        String ch = "meshkeeper-node";
        NotificationManager nm = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (Build.VERSION.SDK_INT >= 26 && nm != null) {
            NotificationChannel c = new NotificationChannel(ch, "Узел MeshKeeper", NotificationManager.IMPORTANCE_LOW);
            c.setDescription("Телефон раздаёт учёт по Wi‑Fi");
            nm.createNotificationChannel(c);
        }
        Intent open = new Intent(this, MainActivity.class);
        PendingIntent pi = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder b;
        if (Build.VERSION.SDK_INT >= 26) b = new Notification.Builder(this, ch);
        else b = new Notification.Builder(this);
        return b.setContentTitle("MeshKeeper")
                .setContentText("Этот телефон — узел учёта · " + NodeRuntime.lanOrigin())
                .setSmallIcon(android.R.drawable.stat_notify_sync)
                .setContentIntent(pi)
                .setOngoing(true)
                .build();
    }

    @Override
    public IBinder onBind(Intent intent) { return null; }
}
