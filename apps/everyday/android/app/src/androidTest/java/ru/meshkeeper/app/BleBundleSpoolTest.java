package ru.meshkeeper.app;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.fail;

import android.content.Context;

import androidx.test.core.app.ApplicationProvider;
import androidx.test.ext.junit.runners.AndroidJUnit4;

import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;

@RunWith(AndroidJUnit4.class)
public class BleBundleSpoolTest {
    private Context context;
    private File directory;

    @Before public void prepare() {
        context = ApplicationProvider.getApplicationContext();
        directory = new File(context.getNoBackupFilesDir(), "ble-spool");
        clear();
    }

    @After public void cleanup() { clear(); }

    @Test public void outgoingSurvivesPreviousStateAndDeletesOnlyAfterDelivery() throws Exception {
        byte[] oldBundle = "old-encrypted-bundle".getBytes(StandardCharsets.UTF_8);
        File previous = new File(directory, "outgoing.bundle.previous");
        if (!directory.mkdirs() && !directory.isDirectory()) fail("spool directory");
        try (FileOutputStream output = new FileOutputStream(previous)) { output.write(oldBundle); }
        assertArrayEquals(oldBundle, BleBundleSpool.peekOutgoing(context));

        byte[] fresh = "fresh-encrypted-bundle".getBytes(StandardCharsets.UTF_8);
        BleBundleSpool.putOutgoing(context, fresh);
        assertArrayEquals(fresh, BleBundleSpool.peekOutgoing(context));
        BleBundleSpool.removeOutgoing(context);
        assertNull(BleBundleSpool.peekOutgoing(context));
    }

    @Test public void incomingUsesClaimRejectRetryAndAccept() throws Exception {
        byte[] bundle = "encrypted-incoming-bundle".getBytes(StandardCharsets.UTF_8);
        BleBundleSpool.putIncoming(context, bundle);
        assertEquals(1, BleBundleSpool.pendingIncoming(context));
        assertEquals(new String(bundle, StandardCharsets.UTF_8), BleBundleSpool.takeIncoming(context));
        assertEquals(1, BleBundleSpool.pendingIncoming(context));

        BleBundleSpool.acknowledgeIncoming(context, false);
        assertEquals(1, BleBundleSpool.pendingIncoming(context));
        assertEquals(new String(bundle, StandardCharsets.UTF_8), BleBundleSpool.takeIncoming(context));
        BleBundleSpool.acknowledgeIncoming(context, true);
        assertEquals(0, BleBundleSpool.pendingIncoming(context));
    }

    @Test public void incomingQueueIsBounded() throws Exception {
        for (int index = 0; index < 4; index++) {
            BleBundleSpool.putIncoming(context, new byte[]{(byte) index});
        }
        try {
            BleBundleSpool.putIncoming(context, new byte[]{9});
            fail("fifth bundle must be rejected");
        } catch (java.io.IOException expected) {
            assertEquals(4, BleBundleSpool.pendingIncoming(context));
        }
    }

    @Test public void corruptClaimIsQuarantinedWithoutRetryLoop() throws Exception {
        if (!directory.mkdirs() && !directory.isDirectory()) fail("spool directory");
        File claimed = new File(directory, "incoming-claimed.bundle");
        if (!claimed.createNewFile()) fail("claimed file");
        try {
            BleBundleSpool.takeIncoming(context);
            fail("empty claim must be rejected");
        } catch (java.io.IOException expected) {
            assertEquals(0, BleBundleSpool.pendingIncoming(context));
            File[] quarantined = directory.listFiles((dir, name) ->
                    name.startsWith("quarantine-corrupt-") && name.endsWith(".bundle"));
            assertEquals(1, quarantined == null ? 0 : quarantined.length);
        }
    }

    @Test public void bleFramingKeepsSyncAndInterorgPayloadKindsSeparate() {
        assertEquals(1, BleMeshTransport.transportKind(
                "{\"format\":\"everyday-sync-bundle\",\"version\":2}"
                        .getBytes(StandardCharsets.UTF_8)));
        assertEquals(3, BleMeshTransport.transportKind(
                "{\"format\":\"everyday-interorg-gossip\",\"version\":1,\"envelopes\":[]}"
                        .getBytes(StandardCharsets.UTF_8)));
        try {
            BleMeshTransport.transportKind(
                    "{\"format\":\"attacker-payload\"}".getBytes(StandardCharsets.UTF_8));
            fail("unknown payload kind must fail closed");
        } catch (IllegalArgumentException expected) {
            // expected
        }
    }

    private void clear() {
        if (!directory.isDirectory()) return;
        File[] files = directory.listFiles();
        if (files != null) for (File file : files) file.delete();
        directory.delete();
    }
}
