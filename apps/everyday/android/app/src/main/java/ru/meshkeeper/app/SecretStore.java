package ru.meshkeeper.app;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;

import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.KeyStore;

import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** Hardware-backed where available, non-exportable storage for Android node secrets. */
final class SecretStore {
    private static final String PREFS = "meshkeeper";
    private static final String LEGACY_TOKEN = "sync_token";
    private static final String TOKEN_CIPHERTEXT = "sync_token_ciphertext_v1";
    private static final String TOKEN_IV = "sync_token_iv_v1";
    private static final String NODE_KEY_CIPHERTEXT = "node_signing_key_ciphertext_v1";
    private static final String NODE_KEY_IV = "node_signing_key_iv_v1";
    private static final String CAPABILITIES_CIPHERTEXT = "sync_capabilities_ciphertext_v1";
    private static final String CAPABILITIES_IV = "sync_capabilities_iv_v1";
    private static final String KEY_ALIAS = "meshkeeper.sync-token.v1";
    private static final String KEYSTORE = "AndroidKeyStore";
    private static final byte[] TOKEN_AAD = aad("sync-token/v1");
    private static final byte[] NODE_KEY_AAD = aad("node-signing-key/v1");
    private static final byte[] CAPABILITIES_AAD = aad("sync-capabilities/v1");

    private SecretStore() {}

    static void saveSyncToken(Context context, String token) throws GeneralSecurityException {
        save(context, TOKEN_CIPHERTEXT, TOKEN_IV, LEGACY_TOKEN, TOKEN_AAD, token);
    }

    static String loadSyncToken(Context context) throws GeneralSecurityException {
        String encrypted = load(context, TOKEN_CIPHERTEXT, TOKEN_IV, TOKEN_AAD);
        if (!encrypted.isEmpty()) return encrypted;
        // One-time migration from builds that used private but plaintext preferences.
        String legacy = preferences(context).getString(LEGACY_TOKEN, "");
        if (!legacy.isEmpty()) {
            saveSyncToken(context, legacy);
            return legacy;
        }
        return "";
    }

    static void saveNodeSigningKey(Context context, String key) throws GeneralSecurityException {
        save(context, NODE_KEY_CIPHERTEXT, NODE_KEY_IV, null, NODE_KEY_AAD, key);
    }

    static String loadNodeSigningKey(Context context) throws GeneralSecurityException {
        return load(context, NODE_KEY_CIPHERTEXT, NODE_KEY_IV, NODE_KEY_AAD);
    }

    static void saveSyncCapabilities(Context context, String capabilitiesJson)
            throws GeneralSecurityException {
        save(context, CAPABILITIES_CIPHERTEXT, CAPABILITIES_IV, null,
                CAPABILITIES_AAD, capabilitiesJson);
    }

    static String loadSyncCapabilities(Context context) throws GeneralSecurityException {
        return load(context, CAPABILITIES_CIPHERTEXT, CAPABILITIES_IV, CAPABILITIES_AAD);
    }

    private static void save(Context context, String ciphertextName, String ivName,
                             String legacyName, byte[] aad, String value)
            throws GeneralSecurityException {
        if (value == null || value.isEmpty()) {
            SharedPreferences.Editor editor = preferences(context).edit()
                    .remove(ciphertextName).remove(ivName);
            if (legacyName != null) editor.remove(legacyName);
            if (!editor.commit()) {
                throw new GeneralSecurityException("Не удалось очистить защищённый секрет");
            }
            return;
        }
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey());
        cipher.updateAAD(aad);
        byte[] encrypted = cipher.doFinal(value.getBytes(StandardCharsets.UTF_8));
        SharedPreferences.Editor editor = preferences(context).edit()
                .putString(ciphertextName, Base64.encodeToString(encrypted, Base64.NO_WRAP))
                .putString(ivName, Base64.encodeToString(cipher.getIV(), Base64.NO_WRAP));
        if (legacyName != null) editor.remove(legacyName);
        if (!editor.commit()) throw new GeneralSecurityException("Не удалось сохранить защищённый секрет");
    }

    private static String load(Context context, String ciphertextName, String ivName, byte[] aad)
            throws GeneralSecurityException {
        SharedPreferences preferences = preferences(context);
        String encrypted = preferences.getString(ciphertextName, "");
        String iv = preferences.getString(ivName, "");
        if (!encrypted.isEmpty() || !iv.isEmpty()) {
            if (encrypted.isEmpty() || iv.isEmpty()) {
                throw new GeneralSecurityException("Повреждено защищённое хранилище");
            }
            try {
                Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
                cipher.init(Cipher.DECRYPT_MODE, getOrCreateKey(),
                        new GCMParameterSpec(128, Base64.decode(iv, Base64.NO_WRAP)));
                cipher.updateAAD(aad);
                return new String(cipher.doFinal(Base64.decode(encrypted, Base64.NO_WRAP)),
                        StandardCharsets.UTF_8);
            } catch (IllegalArgumentException error) {
                throw new GeneralSecurityException("Повреждена кодировка защищённого секрета", error);
            }
        }
        return "";
    }

    private static byte[] aad(String purpose) {
        return ("ru.meshkeeper.app/" + purpose).getBytes(StandardCharsets.UTF_8);
    }

    private static SharedPreferences preferences(Context context) {
        return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
    }

    private static SecretKey getOrCreateKey() throws GeneralSecurityException {
        KeyStore keyStore = KeyStore.getInstance(KEYSTORE);
        try {
            keyStore.load(null);
        } catch (java.io.IOException error) {
            throw new GeneralSecurityException("Android Keystore недоступен", error);
        }
        java.security.Key existing = keyStore.getKey(KEY_ALIAS, null);
        if (existing instanceof SecretKey) return (SecretKey) existing;

        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE);
        generator.init(new KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true)
                .setKeySize(256)
                .build());
        return generator.generateKey();
    }
}
