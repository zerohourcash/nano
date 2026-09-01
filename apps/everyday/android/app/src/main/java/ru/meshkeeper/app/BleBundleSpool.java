package ru.meshkeeper.app;

import android.content.Context;
import android.system.ErrnoException;
import android.system.Os;
import android.system.OsConstants;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.FileDescriptor;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Comparator;
import java.util.UUID;

/** Crash-safe bounded spool for encrypted transport bundles only. */
final class BleBundleSpool {
    static final int MAX_BUNDLE_BYTES = 30 * 1024 * 1024;
    private static final int MAX_INCOMING = 4;
    private static final long MAX_INCOMING_BYTES = 64L * 1024 * 1024;
    private static final String OUTGOING = "outgoing.bundle";
    private static final String CLAIMED = "incoming-claimed.bundle";

    private BleBundleSpool() {}

    static synchronized void putOutgoing(Context context, byte[] bundle) throws IOException {
        validate(bundle);
        atomicWrite(directory(context), OUTGOING, bundle);
    }

    static synchronized byte[] peekOutgoing(Context context) throws IOException {
        File directory = directory(context);
        File target = new File(directory, OUTGOING);
        File previous = new File(directory, OUTGOING + ".previous");
        if (!target.isFile() && previous.isFile()) {
            if (!previous.renameTo(target)) {
                throw new IOException("Не удалось восстановить исходящий BLE spool");
            }
            syncDirectory(directory);
        }
        return readBounded(target);
    }

    static synchronized void removeOutgoing(Context context) throws IOException {
        File directory = directory(context);
        discard(new File(directory, OUTGOING));
        discard(new File(directory, OUTGOING + ".previous"));
        syncDirectory(directory);
    }

    static synchronized void putIncoming(Context context, byte[] bundle) throws IOException {
        validate(bundle);
        File directory = directory(context);
        File[] existing = incoming(directory);
        long bytes = bundle.length;
        for (File file : existing) bytes += file.length();
        File claimed = new File(directory, CLAIMED);
        if (claimed.isFile()) bytes += claimed.length();
        if (existing.length + (claimed.isFile() ? 1 : 0) >= MAX_INCOMING
                || bytes > MAX_INCOMING_BYTES) {
            throw new IOException("Очередь входящих BLE-пакетов заполнена");
        }
        atomicWrite(directory, "incoming-" + System.currentTimeMillis() + "-"
                + UUID.randomUUID() + ".bundle", bundle);
    }

    static synchronized String takeIncoming(Context context) throws IOException {
        File directory = directory(context);
        File claimed = new File(directory, CLAIMED);
        if (!claimed.isFile()) {
            File[] files = incoming(directory);
            if (files.length == 0) return "";
            Arrays.sort(files, Comparator.comparingLong(File::lastModified).thenComparing(File::getName));
            if (!files[0].renameTo(claimed)) throw new IOException("Не удалось занять BLE-пакет");
            syncDirectory(directory);
        }
        try {
            byte[] bytes = readBounded(claimed);
            return new String(bytes, StandardCharsets.UTF_8);
        } catch (IOException error) {
            File corrupt = new File(directory, "quarantine-corrupt-"
                    + System.currentTimeMillis() + "-" + UUID.randomUUID() + ".bundle");
            if (!claimed.renameTo(corrupt)) discard(claimed);
            syncDirectory(directory);
            throw error;
        }
    }

    static synchronized void acknowledgeIncoming(Context context, boolean accepted) throws IOException {
        File directory = directory(context);
        File claimed = new File(directory, CLAIMED);
        if (!claimed.isFile()) return;
        if (accepted) {
            if (!claimed.delete()) throw new IOException("Не удалось подтвердить BLE-пакет");
        } else {
            File retry = new File(directory, "incoming-" + System.currentTimeMillis() + "-"
                    + UUID.randomUUID() + ".bundle");
            if (!claimed.renameTo(retry)) throw new IOException("Не удалось вернуть BLE-пакет в очередь");
        }
        syncDirectory(directory);
    }

    static synchronized int pendingIncoming(Context context) {
        File directory = directory(context);
        return incoming(directory).length + (new File(directory, CLAIMED).isFile() ? 1 : 0);
    }

    private static File directory(Context context) {
        File directory = new File(context.getNoBackupFilesDir(), "ble-spool");
        if (!directory.isDirectory() && !directory.mkdirs()) {
            throw new IllegalStateException("Не удалось создать BLE spool");
        }
        return directory;
    }

    private static File[] incoming(File directory) {
        File[] files = directory.listFiles((dir, name) -> name.startsWith("incoming-")
                && name.endsWith(".bundle") && !CLAIMED.equals(name));
        return files == null ? new File[0] : files;
    }

    private static void validate(byte[] bundle) {
        if (bundle == null || bundle.length == 0 || bundle.length > MAX_BUNDLE_BYTES) {
            throw new IllegalArgumentException("Некорректный размер BLE bundle");
        }
    }

    private static byte[] readBounded(File file) throws IOException {
        if (!file.isFile()) return null;
        long length = file.length();
        if (length <= 0 || length > MAX_BUNDLE_BYTES) throw new IOException("Некорректный BLE spool");
        byte[] result = new byte[(int) length];
        try (FileInputStream input = new FileInputStream(file)) {
            int offset = 0;
            while (offset < result.length) {
                int read = input.read(result, offset, result.length - offset);
                if (read < 0) throw new IOException("Обрезанный BLE spool");
                offset += read;
            }
            if (input.read() != -1) throw new IOException("BLE spool изменился во время чтения");
        }
        return result;
    }

    private static void atomicWrite(File directory, String name, byte[] bytes) throws IOException {
        File target = new File(directory, name);
        File temporary = new File(directory, name + ".tmp-" + UUID.randomUUID());
        try (FileOutputStream output = new FileOutputStream(temporary)) {
            output.write(bytes);
            output.getFD().sync();
        }
        File previous = new File(directory, name + ".previous");
        if (previous.isFile() && !previous.delete()) {
            discard(temporary);
            throw new IOException("Не удалось очистить предыдущий BLE spool");
        }
        if (target.isFile() && !target.renameTo(previous)) {
            discard(temporary);
            throw new IOException("Не удалось подготовить замену BLE spool");
        }
        if (!temporary.renameTo(target)) {
            if (previous.isFile()) previous.renameTo(target);
            discard(temporary);
            throw new IOException("Не удалось опубликовать BLE spool");
        }
        syncDirectory(directory);
        discard(previous);
        syncDirectory(directory);
    }

    private static void discard(File file) {
        if (file.isFile() && !file.delete()) file.deleteOnExit();
    }

    private static void syncDirectory(File directory) throws IOException {
        FileDescriptor descriptor = null;
        try {
            descriptor = Os.open(directory.getAbsolutePath(), OsConstants.O_RDONLY, 0);
            Os.fsync(descriptor);
        } catch (ErrnoException error) {
            throw new IOException("Не удалось синхронизировать BLE spool", error);
        } finally {
            if (descriptor != null) {
                try { Os.close(descriptor); } catch (ErrnoException ignored) {}
            }
        }
    }
}
