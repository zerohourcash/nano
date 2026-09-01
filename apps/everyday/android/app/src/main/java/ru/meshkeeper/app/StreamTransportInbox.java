package ru.meshkeeper.app;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Arrays;
import java.util.HashMap;
import java.util.Map;

/**
 * Bounded Android-side inbox for opaque MKST frames received by BLE/LoRa/etc.
 * It only controls memory and duplicates; Rust remains the framing authority.
 */
final class StreamTransportInbox {
    private static final int HEADER_BYTES = 76;
    private static final int MAX_TRANSFER_BYTES = 32 * 1024 * 1024;
    private static final int MAX_FRAMES = 262_144;
    private static final byte[] MAGIC = new byte[]{'M', 'K', 'S', 'T'};

    private final Map<Integer, byte[]> frames = new HashMap<>();
    private byte[] identity;
    private int frameCount;
    private int totalBytes;
    private int receivedBytes;

    synchronized void accept(byte[] encoded) {
        if (encoded == null || encoded.length < HEADER_BYTES || encoded.length > 65_535) {
            throw new IllegalArgumentException("Некорректный размер transport frame");
        }
        if (!Arrays.equals(Arrays.copyOfRange(encoded, 0, 4), MAGIC) || encoded[4] != 1) {
            throw new IllegalArgumentException("Неизвестный transport frame");
        }
        ByteBuffer header = ByteBuffer.wrap(encoded).order(ByteOrder.BIG_ENDIAN);
        int declaredTotal = header.getInt(22);
        int sequence = header.getInt(26);
        int declaredCount = header.getInt(30);
        int payloadBytes = Short.toUnsignedInt(header.getShort(34));
        if (declaredTotal < 0 || declaredTotal > MAX_TRANSFER_BYTES
                || declaredCount < 1 || declaredCount > MAX_FRAMES
                || (declaredTotal > 0 && declaredCount > declaredTotal)
                || sequence < 0 || sequence >= declaredCount
                || encoded.length != HEADER_BYTES + payloadBytes) {
            throw new IllegalArgumentException("Transport frame вышел за установленные лимиты");
        }
        // kind + transferId + total + count + full digest; sequence/tag are per-frame.
        byte[] declaredIdentity = new byte[1 + 16 + 4 + 4 + 32];
        int cursor = 0;
        declaredIdentity[cursor++] = encoded[5];
        System.arraycopy(encoded, 6, declaredIdentity, cursor, 16); cursor += 16;
        System.arraycopy(encoded, 22, declaredIdentity, cursor, 4); cursor += 4;
        System.arraycopy(encoded, 30, declaredIdentity, cursor, 4); cursor += 4;
        System.arraycopy(encoded, 36, declaredIdentity, cursor, 32);
        if (identity == null) {
            identity = declaredIdentity;
            frameCount = declaredCount;
            totalBytes = declaredTotal;
        } else if (!Arrays.equals(identity, declaredIdentity)) {
            throw new IllegalArgumentException("Смешаны разные transport transfers");
        }
        byte[] existing = frames.get(sequence);
        if (existing != null) {
            if (!Arrays.equals(existing, encoded)) {
                throw new IllegalArgumentException("Конфликтующий повтор transport frame");
            }
            return;
        }
        if ((long) receivedBytes + payloadBytes > totalBytes) {
            throw new IllegalArgumentException("Transport payload превышает объявленный размер");
        }
        // Native parser checks the chunk tag before the frame can be retained.
        RustNode.validateTransportFrame(encoded);
        frames.put(sequence, encoded.clone());
        receivedBytes += payloadBytes;
    }

    synchronized boolean isComplete() {
        return identity != null && frames.size() == frameCount && receivedBytes == totalBytes;
    }

    synchronized String missingRangesJson() {
        return identity == null ? "[]" : RustNode.missingTransportRanges(snapshot());
    }

    synchronized byte[] assemble() {
        if (!isComplete()) throw new IllegalStateException("Transport transfer получен не полностью");
        return RustNode.assembleTransport(snapshot());
    }

    synchronized void reset() {
        frames.clear();
        identity = null;
        frameCount = 0;
        totalBytes = 0;
        receivedBytes = 0;
    }

    private byte[][] snapshot() {
        byte[][] result = new byte[frames.size()][];
        int index = 0;
        for (byte[] frame : frames.values()) result[index++] = frame;
        return result;
    }
}
