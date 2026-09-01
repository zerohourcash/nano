package ru.meshkeeper.app;

import android.annotation.SuppressLint;
import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothDevice;
import android.bluetooth.BluetoothGatt;
import android.bluetooth.BluetoothGattCallback;
import android.bluetooth.BluetoothGattCharacteristic;
import android.bluetooth.BluetoothGattServer;
import android.bluetooth.BluetoothGattServerCallback;
import android.bluetooth.BluetoothGattService;
import android.bluetooth.BluetoothManager;
import android.bluetooth.BluetoothProfile;
import android.bluetooth.BluetoothStatusCodes;
import android.bluetooth.le.AdvertiseCallback;
import android.bluetooth.le.AdvertiseData;
import android.bluetooth.le.AdvertiseSettings;
import android.bluetooth.le.BluetoothLeAdvertiser;
import android.bluetooth.le.BluetoothLeScanner;
import android.bluetooth.le.ScanCallback;
import android.bluetooth.le.ScanFilter;
import android.bluetooth.le.ScanResult;
import android.bluetooth.le.ScanSettings;
import android.content.Context;
import android.os.Build;
import android.os.ParcelUuid;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

/** Opt-in BLE GATT carrier. Payload semantics and verification remain in Rust. */
final class BleMeshTransport {
    interface Listener {
        void onBundle(byte[] bundle);
        void onStatus(String message, boolean error);
    }

    private static final UUID SERVICE_UUID = UUID.fromString("90f15a10-7fe1-4f61-a649-6d6573680001");
    private static final UUID FRAME_UUID = UUID.fromString("90f15a10-7fe1-4f61-a649-6d6573680002");
    private static final int DESIRED_MTU = 185;
    private static final int MAX_REMOTE_INBOXES = 32;
    private static final int MAX_OUTGOING_CONNECTIONS = 8;

    private final Context context;
    private final Listener listener;
    private final BluetoothManager manager;
    private final BluetoothAdapter adapter;
    private final Map<String, StreamTransportInbox> inboxes = new HashMap<>();
    private final Set<String> connecting = new HashSet<>();
    private final Set<String> delivered = new HashSet<>();
    private final Map<BluetoothGatt, SendState> senders = new HashMap<>();
    private final Map<String, byte[][]> retryPlans = new HashMap<>();
    private BluetoothGattServer server;
    private BluetoothLeAdvertiser advertiser;
    private BluetoothLeScanner scanner;
    private byte[] outgoing;
    private boolean advertising;
    private boolean scanning;

    private static final class SendState {
        final BluetoothGattCharacteristic characteristic;
        final byte[][] frames;
        int index;
        SendState(BluetoothGattCharacteristic characteristic, byte[][] frames) {
            this.characteristic = characteristic;
            this.frames = frames;
        }
    }

    BleMeshTransport(Context context, Listener listener) {
        this.context = context.getApplicationContext();
        this.listener = listener;
        manager = (BluetoothManager) context.getSystemService(Context.BLUETOOTH_SERVICE);
        adapter = manager == null ? null : manager.getAdapter();
    }

    @SuppressLint("MissingPermission")
    synchronized void enableReceiver() {
        if (adapter == null || !adapter.isEnabled()) {
            listener.onStatus("Bluetooth выключен или BLE не поддерживается", true);
            return;
        }
        if (server != null) return;
        server = manager.openGattServer(context, serverCallback);
        if (server == null) {
            listener.onStatus("Не удалось открыть BLE GATT server", true);
            return;
        }
        BluetoothGattService service = new BluetoothGattService(
                SERVICE_UUID, BluetoothGattService.SERVICE_TYPE_PRIMARY);
        BluetoothGattCharacteristic frames = new BluetoothGattCharacteristic(
                FRAME_UUID,
                BluetoothGattCharacteristic.PROPERTY_WRITE | BluetoothGattCharacteristic.PROPERTY_WRITE_NO_RESPONSE,
                BluetoothGattCharacteristic.PERMISSION_WRITE);
        service.addCharacteristic(frames);
        if (!server.addService(service)) {
            listener.onStatus("Не удалось зарегистрировать BLE mesh service", true);
            closeServer();
        }
    }

    @SuppressLint("MissingPermission")
    synchronized void send(byte[] encryptedBundle) {
        if (encryptedBundle == null || encryptedBundle.length == 0
                || encryptedBundle.length > 30 * 1024 * 1024) {
            throw new IllegalArgumentException("Некорректный размер BLE bundle");
        }
        outgoing = encryptedBundle.clone();
        delivered.clear();
        retryPlans.clear();
        enableReceiver();
        if (server == null || scanning) return;
        scanner = adapter.getBluetoothLeScanner();
        if (scanner == null) {
            listener.onStatus("BLE scanner недоступен", true);
            return;
        }
        ScanFilter filter = new ScanFilter.Builder().setServiceUuid(new ParcelUuid(SERVICE_UUID)).build();
        ScanSettings settings = new ScanSettings.Builder()
                .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build();
        scanner.startScan(Collections.singletonList(filter), settings, scanCallback);
        scanning = true;
        listener.onStatus("BLE-поиск соседних нод запущен", false);
    }

    @SuppressLint("MissingPermission")
    synchronized void stop() {
        if (scanner != null && scanning) scanner.stopScan(scanCallback);
        scanning = false;
        for (BluetoothGatt gatt : new ArrayList<>(senders.keySet())) gatt.close();
        senders.clear();
        retryPlans.clear();
        connecting.clear();
        stopAdvertising();
        closeServer();
        inboxes.clear();
        outgoing = null;
    }

    @SuppressLint("MissingPermission")
    private void startAdvertising() {
        if (advertising || adapter == null || !adapter.isMultipleAdvertisementSupported()) {
            if (!advertising) listener.onStatus("BLE advertising не поддерживается устройством", true);
            return;
        }
        advertiser = adapter.getBluetoothLeAdvertiser();
        if (advertiser == null) {
            listener.onStatus("BLE advertiser недоступен", true);
            return;
        }
        AdvertiseSettings settings = new AdvertiseSettings.Builder()
                .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_POWER)
                .setConnectable(true).setTimeout(0).build();
        AdvertiseData data = new AdvertiseData.Builder()
                .addServiceUuid(new ParcelUuid(SERVICE_UUID)).setIncludeDeviceName(false).build();
        advertiser.startAdvertising(settings, data, advertiseCallback);
    }

    @SuppressLint("MissingPermission")
    private void stopAdvertising() {
        if (advertiser != null && advertising) advertiser.stopAdvertising(advertiseCallback);
        advertising = false;
        advertiser = null;
    }

    @SuppressLint("MissingPermission")
    private void closeServer() {
        if (server != null) server.close();
        server = null;
    }

    private final AdvertiseCallback advertiseCallback = new AdvertiseCallback() {
        @Override public void onStartSuccess(AdvertiseSettings settingsInEffect) {
            advertising = true;
            listener.onStatus("BLE-приём mesh-пакетов включён", false);
        }
        @Override public void onStartFailure(int errorCode) {
            advertising = false;
            listener.onStatus("Ошибка BLE advertising: " + errorCode, true);
        }
    };

    private final BluetoothGattServerCallback serverCallback = new BluetoothGattServerCallback() {
        @Override public void onServiceAdded(int status, BluetoothGattService service) {
            synchronized (BleMeshTransport.this) {
                if (server == null) return;
            }
            if (status == BluetoothGatt.GATT_SUCCESS) startAdvertising();
            else listener.onStatus("BLE service не добавлен: " + status, true);
        }

        @Override public void onConnectionStateChange(BluetoothDevice device, int status, int newState) {
            // Keep bounded, already-verified frames across a reconnect so the
            // sender can retry the same transfer ID. Entries are removed on
            // completion/error or when BLE is stopped.
        }

        @SuppressLint("MissingPermission")
        @Override public void onCharacteristicWriteRequest(BluetoothDevice device, int requestId,
                BluetoothGattCharacteristic characteristic, boolean preparedWrite,
                boolean responseNeeded, int offset, byte[] value) {
            int response = BluetoothGatt.GATT_SUCCESS;
            try {
                if (!FRAME_UUID.equals(characteristic.getUuid()) || preparedWrite || offset != 0 || value == null) {
                    throw new IllegalArgumentException("unsupported BLE write");
                }
                StreamTransportInbox inbox;
                synchronized (BleMeshTransport.this) {
                    inbox = inboxes.get(device.getAddress());
                    if (inbox == null) {
                        if (inboxes.size() >= MAX_REMOTE_INBOXES) throw new IllegalStateException("too many BLE senders");
                        inbox = new StreamTransportInbox();
                        inboxes.put(device.getAddress(), inbox);
                    }
                }
                inbox.accept(value);
                if (inbox.isComplete()) {
                    byte[] bundle = inbox.assemble();
                    synchronized (BleMeshTransport.this) { inboxes.remove(device.getAddress()); }
                    listener.onBundle(bundle);
                }
            } catch (RuntimeException error) {
                response = BluetoothGatt.GATT_FAILURE;
                synchronized (BleMeshTransport.this) { inboxes.remove(device.getAddress()); }
                listener.onStatus("BLE-кадр отклонён: " + error.getMessage(), true);
            }
            if (responseNeeded && server != null) server.sendResponse(device, requestId, response, 0, null);
        }
    };

    private final ScanCallback scanCallback = new ScanCallback() {
        @Override public void onScanResult(int callbackType, ScanResult result) { connect(result.getDevice()); }
        @Override public void onScanFailed(int errorCode) {
            scanning = false;
            listener.onStatus("Ошибка BLE scan: " + errorCode, true);
        }
    };

    @SuppressLint("MissingPermission")
    private synchronized void connect(BluetoothDevice device) {
        String address = device.getAddress();
        if (outgoing == null || delivered.contains(address) || connecting.contains(address)
                || connecting.size() >= MAX_OUTGOING_CONNECTIONS) return;
        connecting.add(address);
        BluetoothGatt gatt = device.connectGatt(context, false, clientCallback, BluetoothDevice.TRANSPORT_LE);
        if (gatt == null) {
            connecting.remove(address);
            listener.onStatus("Не удалось подключиться к соседней BLE-ноде", true);
        }
    }

    private final BluetoothGattCallback clientCallback = new BluetoothGattCallback() {
        @SuppressLint("MissingPermission")
        @Override public void onConnectionStateChange(BluetoothGatt gatt, int status, int newState) {
            if (status == BluetoothGatt.GATT_SUCCESS && newState == BluetoothProfile.STATE_CONNECTED) {
                gatt.discoverServices();
                return;
            }
            synchronized (BleMeshTransport.this) {
                connecting.remove(gatt.getDevice().getAddress());
                senders.remove(gatt);
            }
            gatt.close();
        }

        @SuppressLint("MissingPermission")
        @Override public void onServicesDiscovered(BluetoothGatt gatt, int status) {
            BluetoothGattService service = status == BluetoothGatt.GATT_SUCCESS
                    ? gatt.getService(SERVICE_UUID) : null;
            BluetoothGattCharacteristic characteristic = service == null ? null : service.getCharacteristic(FRAME_UUID);
            if (characteristic == null || !gatt.requestMtu(DESIRED_MTU)) fail(gatt, "BLE mesh characteristic/MTU недоступны");
        }

        @Override public void onMtuChanged(BluetoothGatt gatt, int mtu, int status) {
            if (status != BluetoothGatt.GATT_SUCCESS || mtu - 3 <= 76) {
                fail(gatt, "BLE MTU слишком мал: " + mtu);
                return;
            }
            byte[] payload;
            synchronized (BleMeshTransport.this) { payload = outgoing == null ? null : outgoing.clone(); }
            if (payload == null) { fail(gatt, "BLE transfer отменён"); return; }
            BluetoothGattCharacteristic characteristic = gatt.getService(SERVICE_UUID).getCharacteristic(FRAME_UUID);
            try {
                String address = gatt.getDevice().getAddress();
                byte[][] frames;
                synchronized (BleMeshTransport.this) {
                    frames = retryPlans.get(address);
                    if (frames == null) {
                        frames = RustNode.fragmentTransport(payload, mtu - 3, 1);
                        retryPlans.put(address, frames);
                    } else if (frames.length > 0 && frames[0].length > mtu - 3) {
                        throw new IllegalStateException("BLE MTU уменьшился; удалённый partial transfer нужно сбросить");
                    }
                    senders.put(gatt, new SendState(characteristic, frames));
                }
                writeNext(gatt);
            } catch (RuntimeException error) {
                fail(gatt, error.getMessage());
            }
        }

        @Override public void onCharacteristicWrite(BluetoothGatt gatt,
                BluetoothGattCharacteristic characteristic, int status) {
            if (status != BluetoothGatt.GATT_SUCCESS) { fail(gatt, "BLE write: " + status); return; }
            SendState state;
            synchronized (BleMeshTransport.this) { state = senders.get(gatt); if (state != null) state.index++; }
            writeNext(gatt);
        }
    };

    @SuppressLint("MissingPermission")
    private void writeNext(BluetoothGatt gatt) {
        SendState state;
        synchronized (this) { state = senders.get(gatt); }
        if (state == null) return;
        if (state.index >= state.frames.length) {
            String address = gatt.getDevice().getAddress();
            synchronized (this) {
                delivered.add(address);
                connecting.remove(address);
                senders.remove(gatt);
                retryPlans.remove(address);
            }
            listener.onStatus("BLE-пакет передан соседней ноде", false);
            gatt.disconnect();
            return;
        }
        boolean queued;
        if (Build.VERSION.SDK_INT >= 33) {
            queued = gatt.writeCharacteristic(state.characteristic, state.frames[state.index],
                    BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT) == BluetoothStatusCodes.SUCCESS;
        } else {
            state.characteristic.setWriteType(BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT);
            state.characteristic.setValue(state.frames[state.index]);
            queued = gatt.writeCharacteristic(state.characteristic);
        }
        if (!queued) fail(gatt, "BLE write queue отклонила кадр");
    }

    @SuppressLint("MissingPermission")
    private void fail(BluetoothGatt gatt, String message) {
        synchronized (this) {
            connecting.remove(gatt.getDevice().getAddress());
            senders.remove(gatt);
        }
        listener.onStatus(message == null ? "Ошибка BLE transport" : message, true);
        gatt.disconnect();
        gatt.close();
    }
}
