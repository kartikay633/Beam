package dev.beam.android.data

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.util.Log
import androidx.annotation.RequiresApi
import dev.beam.core.ServiceAdvert
import java.util.ArrayDeque
import java.util.concurrent.Executors

/**
 * mDNS through Android's `NsdManager` (SPEC.md 4.1). Core still decides what
 * to announce and what a resolved service means; this only moves records
 * between the system service and the node.
 */
class NsdDiscovery(context: Context) : PlatformDiscovery {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val executor = Executors.newSingleThreadExecutor()
    private val lock = Any()

    private var sink: PlatformDiscovery.Sink? = null
    private var discovery: NsdManager.DiscoveryListener? = null
    private var registration: NsdManager.RegistrationListener? = null
    private val watchers = mutableMapOf<String, Any>()

    // Before Android 14, only one resolve may run at a time.
    private val resolveQueue = ArrayDeque<NsdServiceInfo>()
    private var resolving = false

    override fun start(sink: PlatformDiscovery.Sink) = synchronized(lock) {
        if (discovery != null) return
        this.sink = sink
        val listener = object : NsdManager.DiscoveryListener {
            override fun onServiceFound(info: NsdServiceInfo) = watch(info)
            override fun onServiceLost(info: NsdServiceInfo) = lost(info.serviceName)
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Log.w(TAG, "discovery failed to start: $errorCode")
            }
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
        }
        discovery = listener
        nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
    }

    override fun stop() = synchronized(lock) {
        discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        discovery = null
        if (Build.VERSION.SDK_INT >= 34) {
            watchers.values.forEach { runCatching { nsd.unregisterServiceInfoCallback(it as NsdManager.ServiceInfoCallback) } }
        }
        watchers.clear()
        resolveQueue.clear()
        sink = null
        unregister()
    }

    override fun advertise(advert: ServiceAdvert?) = synchronized(lock) {
        unregister()
        if (advert == null) return
        val info = NsdServiceInfo().apply {
            serviceName = advert.instanceName
            serviceType = SERVICE_TYPE
            port = advert.port.toInt()
            advert.txt.forEach { (key, value) -> setAttribute(key, value) }
        }
        val listener = object : NsdManager.RegistrationListener {
            override fun onServiceRegistered(info: NsdServiceInfo) {}
            override fun onServiceUnregistered(info: NsdServiceInfo) {}
            override fun onRegistrationFailed(info: NsdServiceInfo, errorCode: Int) {
                Log.w(TAG, "could not announce: $errorCode")
            }
            override fun onUnregistrationFailed(info: NsdServiceInfo, errorCode: Int) {}
        }
        registration = listener
        nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener)
    }

    private fun unregister() {
        registration?.let { runCatching { nsd.unregisterService(it) } }
        registration = null
    }

    private fun watch(info: NsdServiceInfo) = synchronized(lock) {
        if (sink == null || info.serviceName in watchers) return
        if (Build.VERSION.SDK_INT >= 34) {
            val callback = serviceInfoCallback()
            watchers[info.serviceName] = callback
            nsd.registerServiceInfoCallback(info, executor, callback)
        } else {
            watchers[info.serviceName] = Unit
            resolveQueue.add(info)
            resolveNext()
        }
    }

    private fun lost(name: String) {
        synchronized(lock) {
            val watcher = watchers.remove(name) ?: return
            if (Build.VERSION.SDK_INT >= 34 && watcher is NsdManager.ServiceInfoCallback) {
                runCatching { nsd.unregisterServiceInfoCallback(watcher) }
            }
        }
        sink?.lost(name)
    }

    private fun report(info: NsdServiceInfo, addresses: List<String>) {
        val txt = info.attributes.mapNotNull { (key, value) -> value?.let { key to it.decodeToString() } }.toMap()
        sink?.found(info.serviceName, txt, addresses, info.port)
    }

    @RequiresApi(34)
    private fun serviceInfoCallback() = object : NsdManager.ServiceInfoCallback {
        override fun onServiceUpdated(info: NsdServiceInfo) =
            report(info, info.hostAddresses.mapNotNull { it.hostAddress })
        override fun onServiceLost() {}
        override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {
            Log.w(TAG, "could not watch a service: $errorCode")
        }
        override fun onServiceInfoCallbackUnregistered() {}
    }

    @Suppress("DEPRECATION") // resolveService is the only option before Android 14.
    private fun resolveNext() {
        val next = synchronized(lock) {
            if (resolving) return
            resolveQueue.poll()?.also { resolving = true } ?: return
        }
        nsd.resolveService(next, object : NsdManager.ResolveListener {
            override fun onServiceResolved(info: NsdServiceInfo) {
                report(info, listOfNotNull(info.host?.hostAddress))
                done()
            }

            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
                // Let a later onServiceFound try again.
                synchronized(lock) { watchers.remove(info.serviceName) }
                done()
            }

            private fun done() {
                synchronized(lock) { resolving = false }
                resolveNext()
            }
        })
    }

    private companion object {
        const val TAG = "Beam"
        const val SERVICE_TYPE = "_beam._tcp"
    }
}
