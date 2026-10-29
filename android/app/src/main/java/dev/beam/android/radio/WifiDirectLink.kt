package dev.beam.android.radio

import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pManager
import android.os.Build
import android.os.Looper
import android.util.Log
import kotlin.coroutines.resume
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout

/**
 * Joins a Wi-Fi Direct group a laptop hosts for a transfer (SPEC.md 5), as
 * a client, with the group's name and passphrase from the Bluetooth
 * handshake. From Android 10 that needs no confirmation from the user, and
 * the phone keeps its Wi-Fi network and mobile data meanwhile; so does the
 * laptop. When joining fails, the caller falls back to the hotspot.
 */
class WifiDirectLink(private val context: Context) {
    /** In the group at [address]; [release] leaves it. */
    class Joined(val address: String, private val leave: () -> Unit) {
        fun release() = leave()
    }

    private val manager: WifiP2pManager? = context.getSystemService(WifiP2pManager::class.java)

    val supported: Boolean
        get() = Build.VERSION.SDK_INT >= 29 &&
            manager != null &&
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_DIRECT)

    /** Joins the group called [ssid], whose host is at [hostAddress]. */
    @SuppressLint("MissingPermission") // RadioPermissions covers Wi-Fi Direct too.
    suspend fun join(ssid: String, passphrase: String, hostAddress: String): Joined {
        check(Build.VERSION.SDK_INT >= 29) { "Wi-Fi Direct joining needs Android 10" }
        val manager = checkNotNull(manager) { "this phone has no Wi-Fi Direct" }
        val channel = checkNotNull(manager.initialize(context, Looper.getMainLooper(), null)) { "Wi-Fi Direct is unavailable" }
        val leave = {
            manager.removeGroup(channel, null)
            channel.close()
        }
        try {
            val config = WifiP2pConfig.Builder()
                .setNetworkName(ssid)
                .setPassphrase(passphrase)
                .enablePersistentMode(false)
                .build()
            var failure = connect(manager, channel, config)
            if (failure == WifiP2pManager.BUSY) {
                // A group from earlier (ours, or another app's) is in the way.
                remove(manager, channel)
                failure = connect(manager, channel, config)
            }
            check(failure == null) { "Wi-Fi Direct refused to connect (${reason(failure!!)})" }
            val address = withTimeout(JOIN_TIMEOUT_MS) { addressBeside(hostAddress) }
            return Joined(address, leave)
        } catch (e: Throwable) {
            Log.w(TAG, "joining $ssid failed", e)
            leave()
            throw e
        }
    }

    /** Null on success, or the failure reason. */
    @SuppressLint("MissingPermission")
    private suspend fun connect(manager: WifiP2pManager, channel: WifiP2pManager.Channel, config: WifiP2pConfig): Int? =
        suspendCancellableCoroutine { cont ->
            manager.connect(
                channel,
                config,
                object : WifiP2pManager.ActionListener {
                    override fun onSuccess() {
                        if (cont.isActive) cont.resume(null)
                    }

                    override fun onFailure(reason: Int) {
                        if (cont.isActive) cont.resume(reason)
                    }
                },
            )
        }

    private suspend fun remove(manager: WifiP2pManager, channel: WifiP2pManager.Channel) {
        suspendCancellableCoroutine { cont ->
            manager.removeGroup(
                channel,
                object : WifiP2pManager.ActionListener {
                    override fun onSuccess() {
                        if (cont.isActive) cont.resume(Unit)
                    }

                    override fun onFailure(reason: Int) {
                        if (cont.isActive) cont.resume(Unit)
                    }
                },
            )
        }
    }

    /** Our address once the group's DHCP gives us one on the host's /24. */
    private suspend fun addressBeside(hostAddress: String): String {
        val subnet = hostAddress.substringBeforeLast('.') + "."
        while (true) {
            LocalHotspot.ipv4Addresses()
                .map { it.second }
                .firstOrNull { it.startsWith(subnet) && it != hostAddress }
                ?.let { return it }
            delay(200)
        }
    }

    private fun reason(code: Int) = when (code) {
        WifiP2pManager.P2P_UNSUPPORTED -> "unsupported"
        WifiP2pManager.BUSY -> "busy"
        else -> "error $code"
    }

    companion object {
        const val TAG = "BeamDirect"
        /** How long joining may take before falling back to the hotspot. */
        const val JOIN_TIMEOUT_MS = 15_000L
    }
}
