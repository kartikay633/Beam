package dev.beam.android

import android.app.Application
import dev.beam.android.data.AndroidFileAccess
import dev.beam.android.data.BeamRepository
import dev.beam.android.data.DiscoveryChoice
import dev.beam.android.data.DownloadsSaver
import dev.beam.android.data.IdentityStore
import dev.beam.android.data.NsdDiscovery
import dev.beam.android.data.PrefsSettings
import dev.beam.android.radio.BleRadio
import dev.beam.android.radio.LocalHotspot
import dev.beam.android.radio.NetworkMonitor
import dev.beam.android.radio.RadioPermissions
import dev.beam.android.service.Notifications
import dev.beam.android.share.DirectShare
import dev.beam.core.BeamNode
import dev.beam.core.BleCapabilities
import dev.beam.core.DeviceKind
import dev.beam.core.Discovery
import dev.beam.core.NodeConfig
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

class BeamApp : Application() {
    /** App-wide, so a transfer outlives the activity. */
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    val settings by lazy { PrefsSettings(this) }
    val files by lazy { AndroidFileAccess(this) }

    val repository: BeamRepository by lazy {
        val identity = IdentityStore(this).loadOrCreate()
        val saver = DownloadsSaver(this)
        BeamRepository(
            scope = scope,
            settings = settings,
            createNode = { s, listener ->
                BeamNode(
                    NodeConfig(
                        deviceName = s.deviceName,
                        deviceKind = DeviceKind.PHONE,
                        identitySecret = identity,
                        dataDir = File(filesDir, "beam").path,
                        // Not the cache: partial files must survive for resume.
                        incomingDir = File(filesDir, "incoming").path,
                        visibility = s.visibility,
                        port = 0u,
                        listenAddress = null,
                        discovery = when (s.discovery) {
                            DiscoveryChoice.ANDROID_NSD -> Discovery.PLATFORM
                            DiscoveryChoice.BUILT_IN -> Discovery.BUILT_IN
                        },
                        // Phones host the hotspot when there's no shared network.
                        ble = if (RadioPermissions.hasBluetoothLe(this)) BleCapabilities(hotspot = true, wifiDirect = false) else null,
                    ),
                    listener,
                )
            },
            platformDiscovery = NsdDiscovery(this),
            saveReceived = { paths -> saver.save(paths) },
        ).also { repo ->
            if (RadioPermissions.hasBluetoothLe(this)) {
                repo.radio = BleRadio(this, LocalHotspot(this), scope) {
                    repo.state.value.busy.any { !it.sending }
                }
            }
        }
    }

    private val network by lazy { NetworkMonitor(this) { repository.setLanAddresses(it) } }

    override fun onCreate() {
        super.onCreate()
        Notifications.createChannels(this)
        network.start()
        // Trusted devices become Direct Share targets in the share sheet.
        scope.launch {
            repository.state.map { it.trusted }.distinctUntilChanged().collect { DirectShare.publish(this@BeamApp, it) }
        }
    }
}
