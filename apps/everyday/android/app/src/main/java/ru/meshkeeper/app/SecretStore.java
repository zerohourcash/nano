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

/** Hardware-backed where available, non-exportable storage for the mesh bearer secret. */
final class SecretStore {
    private static final String PREFS = "meshkeeper";
    private static final String LEGACY_TOKEN = "sync_token";
    private static final String TOKEN_CIPHERTEXT = "sync_token_ciphertext_v1";
    private static final String TOKEN_IV = "sync_token_iv_v1";
    private static final String KEY_ALIAS = "meshkeeper.sync-token.v1";
    private static final String KEYSTORE = "AndroidKeyStore";
    private static final byte[] AAD = "ru.meshkeeper.app/sync-token/v1"
            .getBytes(StandardCharsets.UTF_8);

    private SecretStore() {}

    static void saveSyncToken(Context context, String token) throws GeneralSecurityException {
        if (token == null || token.isEmpty()) {
            if (!preferences(context).edit()
                    .remove(TOKEN_CIPHERTEXT).remove(TOKEN_IV).remove(LEGACY_TOKEN).commit()) {
                throw new GeneralSecurityException("Не удалось очистить mesh-токен");
            }
            return;
        }
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey());
        cipher.updateAAD(AAD);
        byte[] encrypted = cipher.doFinal(token.getBytes(StandardCharsets.UTF_8));
        boolean stored = preferences(context).edit()
                .putString(TOKEN_CIPHERTEXT, Base64.encodeToString(encrypted, Base64.NO_WRAP))
                .putString(TOKEN_IV, Base64.encodeToString(cipher.getIV(), Base64.NO_WRAP))
                .remove(LEGACY_TOKEN)
                .commit();
        if (!stored) throw new GeneralSecurityException("Не удалось сохранить mesh-токен");
    }

    static String loadSyncToken(Context context) throws GeneralSecurityException {
        SharedPreferences preferences = preferences(context);
        String encrypted = preferences.getString(TOKEN_CIPHERTEXT, "");
        String iv = preferences.getString(TOKEN_IV, "");
        if (!encrypted.isEmpty() || !iv.isEmpty()) {
            if (encrypted.isEmpty() || iv.isEmpty()) {
                throw new GeneralSecurityException("Повреждено защищённое хранилище mesh-токена");
            }
            try {
                Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
                cipher.init(Cipher.DECRYPT_MODE, getOrCreateKey(),
                        new GCMParameterSpec(128, Base64.decode(iv, Base64.NO_WRAP)));
                cipher.updateAAD(AAD);
                return new String(cipher.doFinal(Base64.decode(encrypted, Base64.NO_WRAP)),
                        StandardCharsets.UTF_8);
            } catch (IllegalArgumentException error) {
                throw new GeneralSecurityException("Повреждена кодировка mesh-токена", error);
            }
        }

        // One-time migration from builds that used private but plaintext preferences.
        String legacy = preferences.getString(LEGACY_TOKEN, "");
        if (!legacy.isEmpty()) {
            saveSyncToken(context, legacy);
            return legacy;
        }
        return "";
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
