package dev.beam.android.radio

import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.net.MacAddress
import android.net.wifi.WpsInfo
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pDevice
import android.net.wifi.p2p.WifiP2pManager
import android.os.Build
import android.os.Looper
import android.util.Log
import kotlin.coroutines.resume
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Connects to a laptop over Wi-Fi Direct for a transfer (SPEC.md 5): the
 * laptop names its Wi-Fi Direct device in the Bluetooth handshake, the
 * phone finds it in discovery and asks to connect (push button), and Beam
 * on the laptop accepts in code. The laptop becomes the group owner.
 * Keeping the phone's existing Wi-Fi at the same time depends on its
 * driver and channel combination; Android may ask to disconnect it.
 *
 * Completed links stay warm briefly for the next transfer. Optional
 * credential records remain compatible with experimental peers; current
 * Windows builds provision through discovery and push-button pairing.
 */
class WifiDirectLink(private val context: Context, private val transferring: () -> Boolean = { false }) {
    /** A transfer's lease on the group; releasing its last lease starts idle cleanup. */
    class Joined(
        val address: String,
        private val leave: () -> Unit,
        val host: String? = null,
        internal val stillConnected: suspend () -> Boolean = { false },
    ) {
        fun release() = leave()
    }

    private val manager: WifiP2pManager? = context.getSystemService(WifiP2pManager::class.java)
    private val generations = P2pGeneration()
    private val setup = Mutex()
    private val cleanup = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private data class Cached(val peer: List<String?>, val link: RetainedLink<Joined>)
    private var cached: Cached? = null

    val supported: Boolean
        get() = Build.VERSION.SDK_INT >= 29 &&
            manager != null &&
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_DIRECT)

    /**
     * Connects to the Wi-Fi Direct device called [peerName] (or at
     * [peerAddress]) the standard way: peer discovery, then a push-button
     * connection that Beam on the laptop accepts in code. Unlike joining a
     * group by name, Windows lets a phone in this way.
     */
    @SuppressLint("MissingPermission")
    suspend fun connectToPeer(peerName: String?, peerAddress: String?, networkName: String? = null, passphrase: String? = null): Joined = setup.withLock {
        val key = listOf(peerName?.lowercase(java.util.Locale.ROOT), peerAddress?.lowercase(java.util.Locale.ROOT), networkName)
        val previous = cached
        if (previous != null && previous.peer == key && previous.link.isOpen) {
            val connected = withTimeoutOrNull(1_500) { previous.link.value.stillConnected() }
            if (connected == true) {
                retain(previous.link)?.let {
                    Log.i(TAG, "reusing established Wi-Fi Direct link")
                    return@withLock it
                }
            } else if (connected == false) {
                // The remote end may have already removed the group while an
                // old receive grace-period lease is still waiting to release.
                previous.link.close()
            }
        }
        check(previous?.link?.inUse != true) { "Another transfer is using Wi-Fi Direct" }
        previous?.link?.close()
        cached = null
        val joined = join(peerName, peerAddress, networkName, passphrase)
        val link = RetainedLink(joined) { it.release() }
        cached = Cached(key, link)
        checkNotNull(retain(link))
    }

    private fun retain(link: RetainedLink<Joined>): Joined? {
        val lease = link.acquire { token ->
            cleanup.launch {
                delay(IDLE_TIMEOUT_MS)
                // mDNS may also rediscover an established P2P address. Keep
                // the interface up while the native engine is transferring.
                while (link.isOpen && transferring()) delay(1_000)
                link.expire(token)
            }
        } ?: return null
        return Joined(lease.value.address, lease::release, lease.value.host)
    }

    /** A successful TCP probe can reuse P2P, but must also own its lifetime. */
    suspend fun retainRoute(localAddress: String, remoteAddress: String, peerName: String): Joined? = setup.withLock {
        val previous = cached ?: return@withLock null
        if (previous.peer.first() != peerName.lowercase(java.util.Locale.ROOT) ||
            previous.link.value.address != localAddress || previous.link.value.host != remoteAddress) return@withLock null
        retain(previous.link)
    }

    fun stop() {
        cleanup.launch { setup.withLock { cached?.link?.close(); cached = null } }
    }

    @SuppressLint("MissingPermission")
    private suspend fun join(peerName: String?, peerAddress: String?, networkName: String?, passphrase: String?): Joined {
        check(peerName != null || peerAddress != null) { "no peer to connect to" }
        val manager = checkNotNull(manager) { "this phone has no Wi-Fi Direct" }
        val channel = checkNotNull(manager.initialize(context, Looper.getMainLooper(), null)) { "Wi-Fi Direct is unavailable" }
        val generation = generations.begin()
        val released = java.util.concurrent.atomic.AtomicBoolean()
        val leave: () -> Unit = {
            if (released.compareAndSet(false, true)) {
                generations.release(generation, current = {
                    runCatching { manager.cancelConnect(channel, null) }
                    runCatching {
                        manager.removeGroup(channel, object : WifiP2pManager.ActionListener {
                            override fun onSuccess() = channel.close()
                            override fun onFailure(reason: Int) = channel.close()
                        })
                    }.onFailure { channel.close() }
                }, obsolete = { channel.close() })
            }
        }
        try {
            remove(manager, channel)
            // Clear a connection attempt left over from last time, which
            // otherwise keeps Wi-Fi Direct busy.
            action { manager.cancelConnect(channel, it) }
            action { manager.stopPeerDiscovery(channel, it) }
            check((networkName == null) == (passphrase == null)) { "incomplete Direct group credentials" }
            val config = if (networkName != null && passphrase != null) {
                Log.i(TAG, "joining credential-based Wi-Fi Direct group")
                WifiP2pConfig.Builder()
                    .setNetworkName(networkName)
                    .setPassphrase(passphrase)
                    .enablePersistentMode(false)
                    .build()
            } else {
                var discovery = action { manager.discoverPeers(channel, it) }
                repeat(5) {
                    if (discovery != WifiP2pManager.BUSY) return@repeat
                    delay(1_000)
                    discovery = action { manager.discoverPeers(channel, it) }
                }
                check(discovery == null) { "Wi-Fi Direct discovery refused (${reason(discovery!!)})" }
                val peer = withTimeout(DISCOVERY_TIMEOUT_MS) {
                    var found: WifiP2pDevice? = null
                    while (found == null) {
                        val peers = peers(manager, channel)
                        Log.i(TAG, "peers: " + peers.joinToString { "${it.deviceName} ${it.deviceAddress} go=${it.isGroupOwner}" })
                        // Windows may advertise as a peer before its group is
                        // formed. Its group-owner flag is not a readiness signal.
                        found = peers.firstOrNull {
                                ((peerAddress != null && it.deviceAddress.equals(peerAddress, ignoreCase = true)) ||
                                    (peerName != null && it.deviceName.equals(peerName, ignoreCase = true)))
                        }
                        if (found == null) delay(1_000)
                    }
                    found
                }
                WifiP2pConfig.Builder()
                    .setDeviceAddress(MacAddress.fromString(peer.deviceAddress))
                    .enablePersistentMode(false)
                    .build()
                    .apply { wps.setup = WpsInfo.PBC; groupOwnerIntent = 0 }
            }
            val failure = connect(manager, channel, config)
            check(failure == null) { "Wi-Fi Direct refused to connect (${reason(failure!!)})" }
            Log.i(TAG, "connecting to ${peerName ?: peerAddress}")
            val owner = withTimeout(GROUP_TIMEOUT_MS) {
                var host: String? = null
                while (host == null) {
                    val info = connectionInfo(manager, channel)
                    if (info?.groupFormed == true && !info.isGroupOwner) host = info.groupOwnerAddress?.hostAddress
                    if (host == null) delay(500)
                }
                host
            }
            Log.i(TAG, "group formed; laptop at $owner")
            val address = withTimeout(JOIN_TIMEOUT_MS) { addressBeside(manager, channel, owner) }
            Log.i(TAG, "connected to ${peerName ?: peerAddress} as $address")
            return Joined(address, leave, owner) {
                val info = connectionInfo(manager, channel)
                info?.groupFormed == true && !info.isGroupOwner && info.groupOwnerAddress?.hostAddress == owner
            }
        } catch (e: Throwable) {
            Log.w(TAG, "connecting to ${peerName ?: peerAddress} failed", e)
            leave()
            throw e
        }
    }

    @SuppressLint("MissingPermission")
    private suspend fun peers(manager: WifiP2pManager, channel: WifiP2pManager.Channel): List<WifiP2pDevice> =
        suspendCancellableCoroutine { cont -> manager.requestPeers(channel) { if (cont.isActive) cont.resume(it?.deviceList?.toList().orEmpty()) } }

    /** Runs a WifiP2pManager call; null on success, or the failure reason. */
    private suspend fun action(call: (WifiP2pManager.ActionListener) -> Unit): Int? =
        suspendCancellableCoroutine { cont ->
            call(
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
    private suspend fun addressBeside(manager: WifiP2pManager, channel: WifiP2pManager.Channel, hostAddress: String): String {
        val subnet = hostAddress.substringBeforeLast('.') + "."
        var tick = 0
        while (true) {
            val addresses = LocalHotspot.ipv4Addresses()
            addresses.map { it.second }.firstOrNull { it.startsWith(subnet) && it != hostAddress }?.let { return it }
            // Once a second, say where the join is (for debugging on devices).
            if (tick++ % 5 == 0) {
                val info = connectionInfo(manager, channel)
                Log.i(TAG, "waiting: group formed ${info?.groupFormed}, owner ${info?.groupOwnerAddress?.hostAddress}, addresses $addresses")
            }
            delay(200)
        }
    }

    private suspend fun connectionInfo(manager: WifiP2pManager, channel: WifiP2pManager.Channel): android.net.wifi.p2p.WifiP2pInfo? =
        suspendCancellableCoroutine { cont -> manager.requestConnectionInfo(channel) { if (cont.isActive) cont.resume(it) } }

    private fun reason(code: Int) = when (code) {
        WifiP2pManager.P2P_UNSUPPORTED -> "unsupported"
        WifiP2pManager.BUSY -> "busy"
        else -> "error $code"
    }

    companion object {
        const val TAG = "BeamDirect"
        /** Keep a completed connection warm for back-to-back transfers. */
        const val IDLE_TIMEOUT_MS = 60_000L
        /** How long joining may take before falling back to the hotspot. */
        const val JOIN_TIMEOUT_MS = 15_000L
        /** How long finding the laptop over Wi-Fi Direct may take. */
        const val DISCOVERY_TIMEOUT_MS = 15_000L
        /** How long the laptop may take to accept and form the group. */
        const val GROUP_TIMEOUT_MS = 30_000L
    }
}
