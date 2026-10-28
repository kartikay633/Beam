package dev.beam.android.service

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.PowerManager
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.lifecycleScope
import dev.beam.android.BeamApp
import dev.beam.android.data.BeamRepository
import dev.beam.android.data.BeamState
import dev.beam.android.data.DiscoveryChoice
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Keeps Beam receiving while the app is in the background (foreground
 * service, type `connectedDevice`), shows progress with Cancel, and holds a
 * Wi-Fi lock and a partial wake lock only while a transfer is moving.
 */
class BeamService : LifecycleService() {
    private val app get() = application as BeamApp
    private val repository get() = app.repository

    private var wifiLock: WifiManager.WifiLock? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private var multicastLock: WifiManager.MulticastLock? = null
    private val notifiedOffers = mutableSetOf<String>()
    private val notifiedFinished = mutableSetOf<String>()

    override fun onCreate() {
        super.onCreate()
        ServiceCompat.startForeground(
            this,
            Notifications.ID_STATUS,
            Notifications.status(this, repository.state.value, System.currentTimeMillis()),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
        )
        lifecycleScope.launch { repository.start() }
        lifecycleScope.launch {
            // Progress changes many times a second; Android drops
            // notification updates beyond a few per second anyway. A
            // StateFlow is conflated, so the delay skips to the latest state.
            repository.state.collect { state ->
                render(state)
                delay(RENDER_INTERVAL_MS)
            }
        }
        lifecycleScope.launch {
            // The "visible to everyone for N min" countdown.
            while (true) {
                delay(30_000)
                render(repository.state.value)
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        val id = intent?.getStringExtra(EXTRA_ID)
        when (intent?.action) {
            ACTION_STOP -> lifecycleScope.launch {
                repository.stop()
                stopSelf()
            }
            ACTION_CANCEL -> id?.let(repository::cancel)
            ACTION_ACCEPT -> id?.let { repository.respond(it, accept = true, trust = false) }
            ACTION_DECLINE -> id?.let { repository.respond(it, accept = false, trust = false) }
        }
        // Not sticky: Android 12+ won't let a restarted service go back to
        // the foreground from the background. Opening the app starts it.
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        releaseTransferLocks()
        multicastLock?.takeIf { it.isHeld }?.release()
        super.onDestroy()
    }

    @SuppressLint("MissingPermission") // POST_NOTIFICATIONS is checked; without it these just don't show.
    private fun render(state: BeamState) {
        val manager = NotificationManagerCompat.from(this)
        val allowed = manager.areNotificationsEnabled()
        if (allowed) manager.notify(Notifications.ID_STATUS, Notifications.status(this, state, System.currentTimeMillis()))

        if (state.busy.isEmpty()) releaseTransferLocks() else acquireTransferLocks()
        val wantMulticast = state.running && app.settings.discovery == DiscoveryChoice.BUILT_IN
        if (wantMulticast) acquireMulticast() else multicastLock?.takeIf { it.isHeld }?.release()

        for (offer in state.offers) {
            if (notifiedOffers.add(offer.offerId) && allowed) {
                manager.notify(offer.offerId.hashCode(), Notifications.offer(this, offer))
            }
        }
        // Answered or timed out: take the prompt down.
        val open = state.offers.map { it.offerId }.toSet()
        notifiedOffers.filterNot { it in open }.forEach {
            manager.cancel(it.hashCode())
            notifiedOffers.remove(it)
        }

        for (t in state.transfers) {
            // Received files: wait until they're in Downloads.
            val ready = t.finished && (t.sending || t.status != BeamRepository.SAVING)
            if (ready && notifiedFinished.add(t.id) && allowed) {
                manager.notify(t.id.hashCode(), Notifications.finished(this, t))
            }
        }
    }

    private fun acquireTransferLocks() {
        if (wifiLock?.isHeld != true) {
            wifiLock = getSystemService(WifiManager::class.java)
                .createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "beam:transfer")
                .apply { setReferenceCounted(false); acquire() }
        }
        if (wakeLock?.isHeld != true) {
            wakeLock = getSystemService(PowerManager::class.java)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "beam:transfer")
                .apply { setReferenceCounted(false); acquire(WAKE_LOCK_LIMIT_MS) }
        }
    }

    private fun releaseTransferLocks() {
        wifiLock?.takeIf { it.isHeld }?.release()
        wakeLock?.takeIf { it.isHeld }?.release()
    }

    /** Core's own mDNS needs multicast packets, which Wi-Fi drops by default. */
    private fun acquireMulticast() {
        if (multicastLock?.isHeld == true) return
        multicastLock = getSystemService(WifiManager::class.java)
            .createMulticastLock("beam:mdns")
            .apply { setReferenceCounted(false); acquire() }
    }

    companion object {
        const val ACTION_STOP = "dev.beam.android.STOP"
        const val ACTION_CANCEL = "dev.beam.android.CANCEL"
        const val ACTION_ACCEPT = "dev.beam.android.ACCEPT"
        const val ACTION_DECLINE = "dev.beam.android.DECLINE"
        const val EXTRA_ID = "id"

        /** A safety net: a wake lock is never held longer than this. */
        private const val WAKE_LOCK_LIMIT_MS = 6 * 60 * 60 * 1000L
        private const val RENDER_INTERVAL_MS = 500L

        /** Starts receiving; call while the app is in the foreground. */
        fun start(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, BeamService::class.java))
        }
    }
}
