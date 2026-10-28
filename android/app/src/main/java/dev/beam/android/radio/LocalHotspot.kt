package dev.beam.android.radio

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import java.net.Inet4Address
import java.net.NetworkInterface
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Android's local-only hotspot: a private Wi-Fi network with a random name
 * and password, no internet, made for one transfer (SPEC.md 5). Shared by
 * everything that needs it: the first [acquire] starts it, the last
 * [release] stops it.
 */
class LocalHotspot(private val context: Context) {
    data class Info(val ssid: String, val passphrase: String, val address: String?)

    private val wifi = context.applicationContext.getSystemService(WifiManager::class.java)
    private val mutex = Mutex()
    private val lock = Any()
    private var reservation: WifiManager.LocalOnlyHotspotReservation? = null
    private var info: Info? = null
    private var users = 0

    suspend fun acquire(): Info = mutex.withLock {
        synchronized(lock) {
            info?.let {
                users++
                return it
            }
        }
        val before = ipv4Addresses().map { it.second }.toSet()
        val started = start()
        val result = try {
            val credentials = credentials(started)
            Info(credentials.first, credentials.second, findHotspotAddress(before))
        } catch (e: Throwable) {
            started.close()
            throw e
        }
        synchronized(lock) {
            reservation = started
            info = result
            users = 1
        }
        result
    }

    fun release() = synchronized(lock) {
        if (users == 0) return
        users--
        if (users == 0) {
            reservation?.close()
            reservation = null
            info = null
        }
    }

    @SuppressLint("MissingPermission") // Checked by RadioPermissions before Beam goes offline.
    private suspend fun start(): WifiManager.LocalOnlyHotspotReservation = suspendCancellableCoroutine { cont ->
        wifi.startLocalOnlyHotspot(
            object : WifiManager.LocalOnlyHotspotCallback() {
                override fun onStarted(reservation: WifiManager.LocalOnlyHotspotReservation) {
                    if (cont.isActive) cont.resume(reservation) else reservation.close()
                }

                override fun onFailed(reason: Int) {
                    val why = when (reason) {
                        ERROR_NO_CHANNEL -> "no Wi-Fi channel is free"
                        ERROR_INCOMPATIBLE_MODE -> "another hotspot or Wi-Fi mode is on"
                        ERROR_TETHERING_DISALLOWED -> "hotspots are disallowed on this phone"
                        else -> "the phone refused (error $reason)"
                    }
                    if (cont.isActive) cont.resumeWithException(IllegalStateException("Couldn't start a hotspot: $why"))
                }

                override fun onStopped() {
                    synchronized(lock) {
                        reservation = null
                        info = null
                        users = 0
                    }
                }
            },
            Handler(Looper.getMainLooper()),
        )
    }

    @Suppress("DEPRECATION") // The only way below Android 11 (and for the SSID below 13).
    private fun credentials(r: WifiManager.LocalOnlyHotspotReservation): Pair<String, String> {
        if (Build.VERSION.SDK_INT >= 30) {
            val config = r.softApConfiguration
            val ssid = if (Build.VERSION.SDK_INT >= 33) config.wifiSsid?.toString() else config.ssid
            return unquote(checkNotNull(ssid) { "the hotspot has no name" }) to checkNotNull(config.passphrase) { "the hotspot has no password" }
        }
        val config = checkNotNull(r.wifiConfiguration) { "the hotspot has no configuration" }
        return unquote(config.SSID) to unquote(config.preSharedKey)
    }

    /**
     * The phone's address on the hotspot, so the other device knows where to
     * connect. Android doesn't say, so look for the new interface; when it
     * can't be found the laptop falls back to the hotspot's gateway.
     */
    private suspend fun findHotspotAddress(before: Set<String>): String? {
        repeat(15) {
            val now = ipv4Addresses()
            val named = now.firstOrNull { (name, _) -> HOTSPOT_INTERFACES.any { name.startsWith(it) } }
            (named ?: now.firstOrNull { it.second !in before })?.let { return it.second }
            delay(200)
        }
        return null
    }

    companion object {
        private val HOTSPOT_INTERFACES = listOf("ap", "swlan", "softap", "wlan1", "wigig")

        /** (interface name, IPv4 address) for every up, non-loopback interface. */
        fun ipv4Addresses(): List<Pair<String, String>> = runCatching {
            NetworkInterface.getNetworkInterfaces().toList()
                .filter { it.isUp && !it.isLoopback }
                .flatMap { nic ->
                    nic.inetAddresses.toList()
                        .filterIsInstance<Inet4Address>()
                        .filter { it.isSiteLocalAddress }
                        .mapNotNull { a -> a.hostAddress?.let { nic.name to it } }
                }
        }.getOrDefault(emptyList())

        fun unquote(s: String): String = s.removeSurrounding("\"")
    }
}
