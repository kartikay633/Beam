package dev.beam.android.radio

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import java.net.Inet4Address

/**
 * Reports this phone's LAN addresses (Wi-Fi and Ethernet) whenever they
 * change. Bluetooth handshakes use them to prefer an existing network over
 * a hotspot.
 */
class NetworkMonitor(context: Context, private val onChange: (List<String>) -> Unit) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val links = mutableMapOf<Network, List<String>>()
    private var callback: ConnectivityManager.NetworkCallback? = null

    fun start(): Unit = synchronized(links) {
        if (callback != null) return
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) = update {
                links[network] = properties.linkAddresses.map { it.address }
                    .filterIsInstance<Inet4Address>()
                    .mapNotNull { it.hostAddress }
            }

            override fun onLost(network: Network) = update { links.remove(network) }
        }
        callback = cb
        val request = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .addTransportType(NetworkCapabilities.TRANSPORT_ETHERNET)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        runCatching { connectivity.registerNetworkCallback(request, cb) }
        Unit
    }

    fun stop(): Unit = synchronized(links) {
        callback?.let { runCatching { connectivity.unregisterNetworkCallback(it) } }
        callback = null
        links.clear()
    }

    private fun update(change: () -> Unit) {
        val all = synchronized(links) {
            change()
            links.values.flatten().distinct()
        }
        onChange(all)
    }
}
