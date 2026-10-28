package dev.beam.android.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import dev.beam.core.generateIdentitySecret
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The device's Noise static key (SPEC.md 6), generated on first launch.
 *
 * Android Keystore can't hold an X25519 key for us to use from Rust, so the
 * secret is encrypted with an AES-256-GCM key that lives in the Keystore
 * (hardware-backed where available, never exportable) and stored as
 * `identity.bin` = 12-byte IV ‖ ciphertext in app-private storage.
 */
class IdentityStore(context: Context) {
    private val file = File(context.filesDir, "identity.bin")

    fun loadOrCreate(): ByteArray {
        if (file.exists()) {
            runCatching { return decrypt(file.readBytes()) }
            // The Keystore key is gone (app data restored to another
            // device, or the user cleared credentials): start over as a new
            // device. Trusted devices will have to pair again.
            file.delete()
        }
        val secret = generateIdentitySecret()
        val tmp = File(file.path + ".tmp")
        tmp.writeBytes(encrypt(secret))
        check(tmp.renameTo(file)) { "could not save the device key" }
        return secret
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    private fun encrypt(secret: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, key()) }
        return cipher.iv + cipher.doFinal(secret)
    }

    private fun decrypt(stored: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, stored, 0, IV_LEN))
        return cipher.doFinal(stored, IV_LEN, stored.size - IV_LEN)
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "beam-identity-wrap"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_LEN = 12
    }
}
