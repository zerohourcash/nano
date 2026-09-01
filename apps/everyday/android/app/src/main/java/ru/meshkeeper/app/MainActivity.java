package ru.meshkeeper.app;

import android.Manifest;
import android.annotation.SuppressLint;
import android.app.Activity;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.SystemClock;
import android.view.View;
import android.webkit.PermissionRequest;
import android.webkit.CookieManager;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.Button;
import android.widget.EditText;
import android.widget.TextView;
import android.widget.Toast;

import java.util.Arrays;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.security.SecureRandom;
import java.nio.charset.StandardCharsets;
import java.util.HashSet;

import androidx.activity.result.ActivityResultLauncher;
import androidx.activity.result.contract.ActivityResultContracts;
import androidx.annotation.NonNull;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.app.ActivityCompat;
import androidx.core.content.ContextCompat;

import com.journeyapps.barcodescanner.ScanContract;
import com.journeyapps.barcodescanner.ScanOptions;


public class MainActivity extends AppCompatActivity {
    private static final String PREFS = "meshkeeper";
    private static final String KEY_RELAY = "relay";
    private static final String KEY_WORKSPACE_SCOPE = "workspace_scope";

    private WebView web;
    private View setup;
    private EditText serverUrl;
    private EditText syncToken;
    private EditText workspaceScope;
    private EditText syncCapabilities;
    private TextView lanHint;
    private boolean hasStoredToken;
    private boolean hasStoredCapabilities;
    private String pendingMode = "join";
    /** Адрес сервера, с которого открыт интерфейс. Пустой — интерфейс не загружен. */
    private String serverOrigin = "";
    private PermissionRequest pendingWebPermission;
    private ValueCallback<Uri[]> fileCallback;
    private volatile String pendingSyncBundle;
    private static final int MAX_SYNC_BUNDLE_BYTES = 30 * 1024 * 1024;

    private final ActivityResultLauncher<ScanOptions> qrLauncher = registerForActivityResult(
            new ScanContract(),
            result -> {
                if (result == null || result.getContents() == null) return;
                String code = result.getContents();
                web.evaluateJavascript(
                        "window.__onNativeQr && window.__onNativeQr(" + org.json.JSONObject.quote(code) + ");",
                        null);
            });

    private final ActivityResultLauncher<Intent> fileLauncher = registerForActivityResult(
            new ActivityResultContracts.StartActivityForResult(),
            result -> {
                Uri[] uris = WebChromeClient.FileChooserParams.parseResult(
                        result.getResultCode() == Activity.RESULT_OK ? Activity.RESULT_OK : result.getResultCode(),
                        result.getData());
                if (fileCallback != null) {
                    fileCallback.onReceiveValue(uris);
                    fileCallback = null;
                }
            });

    @SuppressLint({"SetJavaScriptEnabled", "AddJavascriptInterface"})
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);
        web = findViewById(R.id.web);
        setup = findViewById(R.id.setup);
        serverUrl = findViewById(R.id.serverUrl);
        syncToken = findViewById(R.id.syncToken);
        workspaceScope = findViewById(R.id.workspaceScope);
        syncCapabilities = findViewById(R.id.syncCapabilities);
        lanHint = findViewById(R.id.lanHint);
        Button btnJoin = findViewById(R.id.btnJoin);
        Button btnCreate = findViewById(R.id.btnCreate);
        Button btnClearCapabilities = findViewById(R.id.btnClearCapabilities);

        SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        serverUrl.setText(prefs.getString(KEY_RELAY, ""));
        workspaceScope.setText(prefs.getString(KEY_WORKSPACE_SCOPE, ""));
        try {
            hasStoredToken = !SecretStore.loadSyncToken(this).isEmpty();
            hasStoredCapabilities = !SecretStore.loadSyncCapabilities(this).isEmpty();
            syncToken.setHint(hasStoredToken
                    ? "mesh-токен защищён на устройстве"
                    : "общий mesh-токен (пусто = создать)");
            syncCapabilities.setHint(hasStoredCapabilities
                    ? "набор организаций защищён на устройстве"
                    : "GUID | токен | peer (по строке)");
        } catch (Exception error) {
            hasStoredToken = false;
            Toast.makeText(this, "Не удалось открыть защищённый mesh-токен", Toast.LENGTH_LONG).show();
        }

        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setDatabaseEnabled(true);
        s.setMediaPlaybackRequiresUserGesture(true);
        s.setAllowFileAccess(false);
        s.setAllowContentAccess(false);
        s.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        s.setJavaScriptCanOpenWindowsAutomatically(false);
        s.setSupportMultipleWindows(false);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            s.setSafeBrowsingEnabled(true);
        }
        s.setUserAgentString(s.getUserAgentString() + " MeshKeeperAndroid");
        WebView.setWebContentsDebuggingEnabled(BuildConfig.DEBUG);
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, false);
        web.setLayerType(View.LAYER_TYPE_HARDWARE, null);
        web.addJavascriptInterface(new JsBridge(), "MeshKeeperNative");
        web.setWebViewClient(new WebViewClient() {
            @Override
            public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri target = request.getUrl();
                if (isTrustedLocalOrigin(target)) return false;
                if ("https".equalsIgnoreCase(target.getScheme())) {
                    try {
                        startActivity(new Intent(Intent.ACTION_VIEW, target));
                    } catch (Exception ignored) {
                        Toast.makeText(MainActivity.this, "Не удалось открыть ссылку", Toast.LENGTH_SHORT).show();
                    }
                }
                return true;
            }

            @Override
            public void onPageFinished(WebView view, String url) {
                view.evaluateJavascript(
                        "window.__meshkeeperNodeMode='android-rust';", null);
            }
        });
        web.setWebChromeClient(new WebChromeClient() {
            @Override
            public void onPermissionRequest(PermissionRequest request) {
                runOnUiThread(() -> grantWebCamera(request));
            }

            @Override
            public void onPermissionRequestCanceled(PermissionRequest request) {
                pendingWebPermission = null;
            }

            @Override
            public boolean onShowFileChooser(WebView webView, ValueCallback<Uri[]> filePathCallback, FileChooserParams fileChooserParams) {
                if (fileCallback != null) fileCallback.onReceiveValue(null);
                fileCallback = filePathCallback;
                Intent intent = fileChooserParams.createIntent();
                try {
                    fileLauncher.launch(Intent.createChooser(intent, "Фото QR"));
                    return true;
                } catch (Exception e) {
                    fileCallback = null;
                    return false;
                }
            }
        });

        btnJoin.setOnClickListener(v -> openApp("join"));
        btnCreate.setOnClickListener(v -> openApp("register"));
        btnClearCapabilities.setOnClickListener(v -> {
            try {
                SecretStore.saveSyncCapabilities(this, "");
                hasStoredCapabilities = false;
                syncCapabilities.setText("");
                syncCapabilities.setHint("GUID | токен | peer (по строке)");
                Toast.makeText(this, "Набор capability удалён", Toast.LENGTH_SHORT).show();
            } catch (Exception error) {
                Toast.makeText(this, "Не удалось удалить capability", Toast.LENGTH_LONG).show();
            }
        });

        showSetupHint();
        askNotify();
        captureIncomingBundle(getIntent());
    }

    private void grantWebCamera(PermissionRequest request) {
        boolean requestsCamera = Arrays.asList(request.getResources())
                .contains(PermissionRequest.RESOURCE_VIDEO_CAPTURE);
        if (!isTrustedLocalOrigin(request.getOrigin()) || !requestsCamera) {
            request.deny();
            return;
        }
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            pendingWebPermission = request;
            ActivityCompat.requestPermissions(this, new String[]{Manifest.permission.CAMERA}, 44);
            return;
        }
        request.grant(new String[]{PermissionRequest.RESOURCE_VIDEO_CAPTURE});
    }

    public void startNativeScan() {
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            pendingMode = "scan";
            ActivityCompat.requestPermissions(this, new String[]{Manifest.permission.CAMERA}, 45);
            return;
        }
        ScanOptions options = new ScanOptions();
        options.setDesiredBarcodeFormats(ScanOptions.QR_CODE);
        options.setPrompt("Наведите на QR-приглашение группы");
        options.setBeepEnabled(false);
        options.setOrientationLocked(false);
        options.setCameraId(0);
        options.setCaptureActivity(com.journeyapps.barcodescanner.CaptureActivity.class);
        qrLauncher.launch(options);
    }

    private class JsBridge {
        @android.webkit.JavascriptInterface
        public String lanOrigin() {
            return RustNode.localOrigin();
        }

        @android.webkit.JavascriptInterface
        public String localOrigin() {
            return serverOrigin;
        }

        @android.webkit.JavascriptInterface
        public void scanQr() {
            runOnUiThread(MainActivity.this::startNativeScan);
        }

        @android.webkit.JavascriptInterface
        public String takePendingSyncBundle() {
            String bundle = pendingSyncBundle;
            pendingSyncBundle = null;
            return bundle == null ? "" : bundle;
        }
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        captureIncomingBundle(intent);
    }

    @SuppressWarnings("deprecation")
    private void captureIncomingBundle(Intent intent) {
        if (intent == null) return;
        Uri uri = Intent.ACTION_SEND.equals(intent.getAction())
                ? intent.getParcelableExtra(Intent.EXTRA_STREAM)
                : Intent.ACTION_VIEW.equals(intent.getAction()) ? intent.getData() : null;
        if (uri == null || !"content".equalsIgnoreCase(uri.getScheme())) return;
        new Thread(() -> {
            try (InputStream input = getContentResolver().openInputStream(uri);
                 ByteArrayOutputStream output = new ByteArrayOutputStream()) {
                if (input == null) throw new IllegalArgumentException("Файл недоступен");
                byte[] buffer = new byte[64 * 1024];
                int total = 0;
                int read;
                while ((read = input.read(buffer)) >= 0) {
                    total += read;
                    if (total > MAX_SYNC_BUNDLE_BYTES) {
                        throw new IllegalArgumentException("Пакет превышает лимит 30 МБ");
                    }
                    output.write(buffer, 0, read);
                }
                String json = output.toString(StandardCharsets.UTF_8.name());
                org.json.JSONObject parsed = new org.json.JSONObject(json);
                if (!"everyday-sync-bundle".equals(parsed.optString("format"))) {
                    throw new IllegalArgumentException("Это не пакет Everyday");
                }
                pendingSyncBundle = json;
                runOnUiThread(() -> {
                    Toast.makeText(this, "Пакет принят — откройте «Офлайн-узлы» для проверки", Toast.LENGTH_LONG).show();
                    web.evaluateJavascript("window.dispatchEvent(new Event('meshkeeper-native-bundle'));", null);
                });
            } catch (Exception error) {
                runOnUiThread(() -> Toast.makeText(this,
                        "Не удалось принять пакет: " + error.getMessage(), Toast.LENGTH_LONG).show());
            }
        }, "meshkeeper-shared-bundle").start();
    }

    private void showSetupHint() {
        String saved = getSharedPreferences(PREFS, MODE_PRIVATE).getString(KEY_RELAY, "");
        lanHint.setText(saved.isEmpty()
                ? "Автономный режим: база и Rust-узел находятся на этом телефоне"
                : "Локальный узел синхронизируется с: " + saved);
    }

    private void askNotify() {
        if (Build.VERSION.SDK_INT >= 33) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                ActivityCompat.requestPermissions(this, new String[]{Manifest.permission.POST_NOTIFICATIONS}, 43);
            }
        }
    }

    private void openApp(String mode) {
        pendingMode = mode;
        loadWeb(mode);
    }

    private void loadWeb(String mode) {
        String relay;
        try {
            relay = normalizeRelay(serverUrl.getText().toString());
        } catch (IllegalArgumentException e) {
            Toast.makeText(this, e.getMessage(), Toast.LENGTH_LONG).show();
            return;
        }
        String token = syncToken.getText().toString().trim();
        if (token.isEmpty() && hasStoredToken) {
            try {
                token = SecretStore.loadSyncToken(this);
            } catch (Exception error) {
                Toast.makeText(this, "Не удалось открыть защищённый mesh-токен", Toast.LENGTH_LONG).show();
                return;
            }
        }
        if (token.isEmpty()) token = randomToken();
        if (token.length() < 32) {
            Toast.makeText(this, "Mesh-токен должен содержать не менее 32 символов", Toast.LENGTH_LONG).show();
            return;
        }
        String scope;
        String capabilitiesJson = "";
        try {
            scope = normalizeWorkspaceScope(workspaceScope.getText().toString());
            String enteredCapabilities = syncCapabilities.getText().toString().trim();
            if (!enteredCapabilities.isEmpty()) {
                capabilitiesJson = normalizeCapabilities(enteredCapabilities);
            } else if (hasStoredCapabilities) {
                capabilitiesJson = SecretStore.loadSyncCapabilities(this);
            }
        } catch (IllegalArgumentException error) {
            Toast.makeText(this, error.getMessage(), Toast.LENGTH_LONG).show();
            return;
        } catch (Exception error) {
            Toast.makeText(this, "Не удалось открыть защищённые capability", Toast.LENGTH_LONG).show();
            return;
        }
        // Multi-capability mode owns its peers and scopes. Do not retain an
        // unused legacy secret or silently mix the legacy relay with it.
        if (!capabilitiesJson.isEmpty()) {
            relay = "";
            scope = "";
            token = "";
        }
        try {
            SecretStore.saveSyncToken(this, token);
            SecretStore.saveSyncCapabilities(this, capabilitiesJson);
        } catch (Exception error) {
            Toast.makeText(this, "Не удалось защитить mesh-токен в Android Keystore", Toast.LENGTH_LONG).show();
            return;
        }
        hasStoredToken = !token.isEmpty();
        hasStoredCapabilities = !capabilitiesJson.isEmpty();
        getSharedPreferences(PREFS, MODE_PRIVATE).edit()
                .putString(KEY_RELAY, relay)
                .putString(KEY_WORKSPACE_SCOPE, scope)
                .apply();
        syncToken.setText("");
        syncToken.setHint(hasStoredToken
                ? "mesh-токен защищён на устройстве"
                : "общий mesh-токен (для одной организации)");
        syncCapabilities.setText("");
        syncCapabilities.setHint(hasStoredCapabilities
                ? "набор организаций защищён на устройстве"
                : "GUID | токен | peer (по строке)");
        Intent service = new Intent(this, NodeService.class)
                .putExtra(NodeService.EXTRA_RELAY, relay);
        ContextCompat.startForegroundService(this, service);
        serverOrigin = RustNode.localOrigin();
        setup.setVisibility(View.GONE);
        web.setVisibility(View.VISIBLE);
        waitForNodeAndLoad(mode);
    }

    private void waitForNodeAndLoad(String mode) {
        new Thread(() -> {
            String error = "Локальный узел не запустился";
            for (int attempt = 0; attempt < 100; attempt++) {
                try {
                    HttpURLConnection connection = (HttpURLConnection)
                            new URL(RustNode.localOrigin() + "/health").openConnection();
                    connection.setConnectTimeout(500);
                    connection.setReadTimeout(500);
                    if (connection.getResponseCode() == 200) {
                        runOnUiThread(() -> web.loadUrl(
                                RustNode.localOrigin() + "/login?app=1&mode=" + mode));
                        return;
                    }
                } catch (Exception exception) {
                    error = exception.getMessage();
                }
                SystemClock.sleep(100);
            }
            String message = error;
            runOnUiThread(() -> {
                Toast.makeText(this, "Ошибка Rust-узла: " + message, Toast.LENGTH_LONG).show();
                web.setVisibility(View.GONE);
                setup.setVisibility(View.VISIBLE);
            });
        }, "meshkeeper-health-wait").start();
    }

    private static String randomToken() {
        byte[] bytes = new byte[32];
        new SecureRandom().nextBytes(bytes);
        StringBuilder out = new StringBuilder(64);
        for (byte value : bytes) out.append(String.format("%02x", value & 0xff));
        return out.toString();
    }

    private static String normalizeWorkspaceScope(String raw) {
        String trimmed = raw == null ? "" : raw.trim();
        if (trimmed.isEmpty()) return "";
        java.util.LinkedHashSet<String> unique = new java.util.LinkedHashSet<>();
        for (String value : trimmed.split(",")) {
            String guid = value.trim();
            if (guid.isEmpty() || guid.length() > 128 || !guid.matches("[A-Za-z0-9_-]+")) {
                throw new IllegalArgumentException("Некорректный GUID организации: " + guid);
            }
            unique.add(guid);
            if (unique.size() > 100) throw new IllegalArgumentException("Разрешено не более 100 организаций");
        }
        return String.join(",", unique);
    }

    /** Human-editable input is converted to the exact fail-closed Rust capability JSON. */
    private static String normalizeCapabilities(String raw) {
        org.json.JSONArray result = new org.json.JSONArray();
        HashSet<String> tokens = new HashSet<>();
        HashSet<String> workspaces = new HashSet<>();
        String[] lines = raw.replace("\r", "").split("\n");
        if (lines.length > 100) throw new IllegalArgumentException("Разрешено не более 100 capability");
        for (String source : lines) {
            String line = source.trim();
            if (line.isEmpty()) continue;
            String[] fields = line.split("\\|", -1);
            if (fields.length < 2 || fields.length > 3) {
                throw new IllegalArgumentException("Формат строки: GUID | токен | peer");
            }
            String guid = normalizeWorkspaceScope(fields[0]);
            if (guid.isEmpty() || guid.contains(",")) {
                throw new IllegalArgumentException("В capability укажите один GUID организации");
            }
            String token = fields[1].trim();
            if (token.length() < 32 || token.length() > 256) {
                throw new IllegalArgumentException("Capability-токен должен содержать 32–256 символов");
            }
            if (!tokens.add(token)) throw new IllegalArgumentException("Capability-токен повторяется");
            if (!workspaces.add(guid)) throw new IllegalArgumentException("GUID организации повторяется");
            org.json.JSONArray peers = new org.json.JSONArray();
            if (fields.length == 3 && !fields[2].trim().isEmpty()) {
                for (String peerValue : fields[2].split(",")) {
                    String peer = normalizeRelay(peerValue);
                    if (peer.isEmpty()) continue;
                    peers.put(peer);
                    if (peers.length() > 32) throw new IllegalArgumentException("Не более 32 peers на capability");
                }
            }
            try {
                result.put(new org.json.JSONObject()
                        .put("token", token)
                        .put("workspaces", new org.json.JSONArray().put(guid))
                        .put("peers", peers));
            } catch (org.json.JSONException error) {
                throw new IllegalArgumentException("Не удалось собрать capability", error);
            }
        }
        if (result.length() == 0) throw new IllegalArgumentException("Capability-набор пуст");
        return result.toString();
    }

    @Override
    public void onBackPressed() {
        if (web.getVisibility() == View.VISIBLE && web.canGoBack()) {
            web.goBack();
            return;
        }
        if (web.getVisibility() == View.VISIBLE) {
            web.setVisibility(View.GONE);
            setup.setVisibility(View.VISIBLE);
            showSetupHint();
            return;
        }
        super.onBackPressed();
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, @NonNull String[] permissions, @NonNull int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        boolean granted = grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED;
        if (requestCode == 44 && pendingWebPermission != null) {
            if (granted && isTrustedLocalOrigin(pendingWebPermission.getOrigin())) {
                pendingWebPermission.grant(new String[]{PermissionRequest.RESOURCE_VIDEO_CAPTURE});
            }
            else pendingWebPermission.deny();
            pendingWebPermission = null;
            return;
        }
        if (requestCode == 45 && granted) startNativeScan();
    }

    private boolean isTrustedLocalOrigin(Uri uri) {
        if (uri == null || serverOrigin.isEmpty()) return false;
        Uri trusted = Uri.parse(serverOrigin);
        return "http".equalsIgnoreCase(uri.getScheme())
                && uri.getHost() != null
                && uri.getHost().equalsIgnoreCase(trusted.getHost())
                && uri.getPort() == trusted.getPort();
    }

    private static String normalizeRelay(String raw) {
        String value = raw == null ? "" : raw.trim().replaceAll("/+$", "");
        if (value.isEmpty()) return "";
        if (!value.contains("://")) value = "https://" + value;
        Uri uri = Uri.parse(value);
        if (!"https".equalsIgnoreCase(uri.getScheme()) || uri.getHost() == null) {
            throw new IllegalArgumentException("Сервер синхронизации должен использовать HTTPS");
        }
        return uri.toString();
    }
}
