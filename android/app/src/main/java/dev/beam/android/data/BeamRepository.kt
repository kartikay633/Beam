package dev.beam.android.data

import dev.beam.android.model.TransferReducer
import dev.beam.android.model.TransferUi
import dev.beam.android.model.TransferUi.Phase
import dev.beam.core.BeamException
import dev.beam.core.BeamListener
import dev.beam.core.BeamNodeInterface
import dev.beam.core.BleAdvertisement
import dev.beam.core.Direction
import dev.beam.core.IncomingOffer
import dev.beam.core.NearbyDevice
import dev.beam.core.NearbyLink
import dev.beam.core.OutgoingFile
import dev.beam.core.ServiceAdvert
import dev.beam.core.TransferInfo
import dev.beam.core.TransferResult
import dev.beam.core.TrustedDevice
import dev.beam.core.Visibility
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Everything the UI and the service show. */
data class BeamState(
    val running: Boolean = false,
    val starting: Boolean = false,
    val error: String? = null,
    val deviceName: String = "",
    val deviceId: String = "",
    val visibility: Visibility = Visibility.TRUSTED_ONLY,
    /** When "everyone" falls back to trusted-only, in epoch milliseconds. */
    val everyoneUntilMs: Long? = null,
    val nearby: List<NearbyDevice> = emptyList(),
    val offers: List<IncomingOffer> = emptyList(),
    val transfers: List<TransferUi> = emptyList(),
    val trusted: List<TrustedDevice> = emptyList(),
    /** A one-off message ("Paired with ThinkPad"), cleared by the UI. */
    val message: String? = null,
) {
    /** Transfers that are moving data (or about to): hold the Wi-Fi and wake locks. */
    val busy: List<TransferUi>
        get() = transfers.filter {
            it.phase == Phase.WAITING || it.phase == Phase.ACTIVE || (it.phase == Phase.INTERRUPTED && it.sending)
        }
}

/** Android's mDNS stack, when it does discovery instead of core (see [DiscoveryChoice]). */
interface PlatformDiscovery {
    interface Sink {
        fun found(serviceName: String, txt: Map<String, String>, addresses: List<String>, port: Int)
        fun lost(serviceName: String)
    }

    fun start(sink: Sink)
    fun advertise(advert: ServiceAdvert?)
    fun stop()
}

/**
 * Bluetooth LE, for devices that share no network (SPEC.md 4.2): advertising
 * and scanning feed the node, and [connect] runs the GATT handshake that
 * sets up a Wi-Fi link to a device heard only over Bluetooth.
 */
interface OfflineRadio {
    /** Permissions granted, Bluetooth on. */
    val available: Boolean

    fun start(node: BeamNodeInterface)
    fun stop()

    /** Re-checks permissions and the Bluetooth switch. */
    fun refresh()
    fun advertise(advert: BleAdvertisement?)

    /** Sets up a Wi-Fi link to [device]; reports progress through [status]. */
    suspend fun connect(device: NearbyDevice, status: (String) -> Unit): OfflineLink
}

/** Where to send after a Bluetooth handshake; [release] once the send is over. */
class OfflineLink(val address: String, val deviceId: String?, val release: () -> Unit)

/**
 * Owns the Rust node and turns its events into [BeamState]. Listener calls
 * arrive on the node's threads; [MutableStateFlow.update] keeps that safe.
 */
class BeamRepository(
    private val scope: CoroutineScope,
    private val settings: SettingsStore,
    private val createNode: (SettingsStore, BeamListener) -> BeamNodeInterface,
    private val platformDiscovery: PlatformDiscovery?,
    /** Moves received files (app-private paths) to Downloads; returns their URIs. */
    private val saveReceived: suspend (List<String>) -> List<String>,
    private val clock: () -> Long = System::currentTimeMillis,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    /** Set once the app is built; it needs this repository's state. */
    var radio: OfflineRadio? = null,
) {
    private val _state = MutableStateFlow(BeamState(deviceName = settings.deviceName))
    val state: StateFlow<BeamState> = _state.asStateFlow()

    private val lifecycle = Mutex()
    private var node: BeamNodeInterface? = null
    private var everyoneJob: Job? = null
    @Volatile private var lanAddresses: List<String> = emptyList()
    /** Bluetooth sends still setting up their link, so Cancel can stop them. */
    private val linking = ConcurrentHashMap<String, Job>()

    /** Starts receiving and discovery. Does nothing if already running. */
    suspend fun start() = lifecycle.withLock {
        if (node != null) return@withLock
        _state.update { it.copy(starting = true, error = null) }
        try {
            withContext(io) {
                val created = createNode(settings, listener)
                created.setStreams(settings.streams.toUInt())
                created.setLanAddresses(lanAddresses)
                created.start()
                node = created
                if (settings.discovery == DiscoveryChoice.ANDROID_NSD) platformDiscovery?.start(sink)
                radio?.start(created)
                _state.update {
                    it.copy(
                        running = true,
                        deviceId = created.deviceId(),
                        deviceName = created.deviceName(),
                        trusted = created.trustedDevices(),
                    )
                }
                visibilityChanged(created.visibility())
            }
        } catch (e: Exception) {
            node?.let(::close)
            node = null
            _state.update { it.copy(error = describe(e)) }
        } finally {
            _state.update { it.copy(starting = false) }
        }
    }

    /** Stops receiving and discovery. Transfers in progress are cut. */
    suspend fun stop() = lifecycle.withLock {
        val current = node ?: return@withLock
        node = null
        withContext(io) {
            radio?.stop()
            platformDiscovery?.stop()
            current.stop()
            close(current)
        }
        everyoneJob?.cancel()
        _state.update { it.copy(running = false, nearby = emptyList(), offers = emptyList()) }
    }

    /** Applies settings that need a new node (name, discovery). */
    suspend fun restart() {
        val wasRunning = node != null
        stop()
        _state.update { it.copy(deviceName = settings.deviceName) }
        if (wasRunning) start()
    }

    fun setVisibility(visibility: Visibility) {
        settings.visibility = visibility
        val current = node ?: return run { _state.update { it.copy(visibility = visibility) } }
        current.setVisibility(visibility)
        visibilityChanged(visibility)
    }

    fun setStreams(streams: Int) {
        settings.streams = streams
        node?.setStreams(streams.toUInt())
    }

    /** This phone's LAN addresses changed (see NetworkMonitor). */
    fun setLanAddresses(addresses: List<String>) {
        lanAddresses = addresses
        node?.setLanAddresses(addresses)
    }

    /** Bluetooth permissions or the Bluetooth switch changed. */
    fun refreshRadio() {
        radio?.refresh()
    }

    /**
     * Sends [files] to [device]: straight away when it's on the network, or
     * through a Bluetooth handshake and a Wi-Fi link when it isn't. Returns
     * the transfer id.
     */
    fun send(device: NearbyDevice, files: List<OutgoingFile>, cleanup: () -> Unit = {}): String {
        val id = UUID.randomUUID().toString()
        val current = node
        val job = scope.launch(io) {
            try {
                checkNotNull(current) { "Beam isn't running" }
                if (device.link == NearbyLink.BLUETOOTH) {
                    sendOffline(current, id, device, files)
                } else {
                    try {
                        current.send(id, device.deviceId, files)
                    } catch (e: BeamException) {
                        // Its LAN addresses didn't answer (a network that
                        // keeps devices apart, or stale addresses): try
                        // Bluetooth if it's in range.
                        val route = current.bluetoothRoute(device.deviceId)
                        if (route == null || radio?.available != true) throw e
                        linking[id] = coroutineContext[Job]!!
                        sendOffline(current, id, route, files)
                    }
                }
            } catch (e: Exception) {
                if (e is kotlinx.coroutines.CancellationException) {
                    _state.update { s -> s.copy(transfers = TransferReducer.finished(s.transfers, id, Phase.CANCELLED, null)) }
                } else {
                    failSend(id, device.name, files, describe(e))
                }
            } finally {
                linking.remove(id)
                cleanup()
            }
        }
        if (device.link == NearbyLink.BLUETOOTH) linking[id] = job
        return id
    }

    private suspend fun sendOffline(node: BeamNodeInterface, id: String, device: NearbyDevice, files: List<OutgoingFile>) {
        val radio = checkNotNull(radio) { "This phone has no Bluetooth LE" }
        _state.update { s ->
            val row = TransferUi(id, true, device.name, files.take(5).map { it.name }, files.size, 0, status = "Connecting over Bluetooth…")
            s.copy(transfers = TransferReducer.started(s.transfers, row).map { if (it.id == id) it.copy(phase = Phase.WAITING) else it })
        }
        val link = radio.connect(device) { status ->
            _state.update { s -> s.copy(transfers = TransferReducer.status(s.transfers, id, status, null)) }
        }
        try {
            // From here the node reports progress like any other send.
            linking.remove(id)
            node.sendTo(id, link.address, link.deviceId, device.name, files)
        } finally {
            link.release()
        }
    }

    fun cancel(transferId: String) {
        linking.remove(transferId)?.cancel() ?: node?.cancel(transferId)
    }

    fun respond(offerId: String, accept: Boolean, trust: Boolean) {
        _state.update { s -> s.copy(offers = s.offers.filterNot { it.offerId == offerId }) }
        try {
            node?.respondOffer(offerId, accept, trust)
        } catch (e: Exception) {
            showMessage(describe(e))
        }
        if (accept && trust) refreshTrusted()
    }

    /** Pairs with the desktop whose QR code was scanned. */
    suspend fun pair(code: String): Result<TrustedDevice> = withContext(io) {
        val current = node ?: return@withContext Result.failure(IllegalStateException("Beam isn't running"))
        try {
            val device = current.pair(code)
            refreshTrusted()
            Result.success(device)
        } catch (e: Exception) {
            Result.failure(IllegalStateException(describe(e)))
        }
    }

    fun setAutoAccept(deviceId: String, autoAccept: Boolean) = guarded { it.setAutoAccept(deviceId, autoAccept) }

    fun removeTrusted(deviceId: String) = guarded { it.removeTrusted(deviceId) }

    fun clearFinished() = _state.update { it.copy(transfers = TransferReducer.clearFinished(it.transfers)) }

    fun showMessage(message: String) = _state.update { it.copy(message = message) }

    fun messageShown() = _state.update { it.copy(message = null) }

    private fun guarded(action: (BeamNodeInterface) -> Unit) {
        val current = node ?: return
        try {
            action(current)
        } catch (e: Exception) {
            showMessage(describe(e))
        }
        refreshTrusted()
    }

    private fun refreshTrusted() {
        val current = node ?: return
        _state.update { it.copy(trusted = current.trustedDevices()) }
    }

    private fun visibilityChanged(visibility: Visibility) {
        everyoneJob?.cancel()
        if (visibility != Visibility.EVERYONE) {
            _state.update { it.copy(visibility = visibility, everyoneUntilMs = null) }
            return
        }
        val until = clock() + EVERYONE_FOR_MS
        _state.update { it.copy(visibility = visibility, everyoneUntilMs = until) }
        // Core falls back to trusted-only on its own; catch up when it does.
        everyoneJob = scope.launch {
            delay(EVERYONE_FOR_MS + 1_000)
            val now = node?.visibility() ?: Visibility.TRUSTED_ONLY
            settings.visibility = now
            _state.update { it.copy(visibility = now, everyoneUntilMs = null) }
        }
    }

    private fun failSend(id: String, targetName: String, files: List<OutgoingFile>, message: String) =
        _state.update { s ->
            val known = s.transfers.any { it.id == id }
            val list = if (known) s.transfers else TransferReducer.started(
                s.transfers,
                TransferUi(id, true, targetName, files.take(5).map { it.name }, files.size, 0),
            )
            s.copy(transfers = TransferReducer.finished(list, id, Phase.FAILED, message))
        }

    private val sink = object : PlatformDiscovery.Sink {
        override fun found(serviceName: String, txt: Map<String, String>, addresses: List<String>, port: Int) {
            node?.serviceFound(serviceName, txt, addresses, port.toUShort())
        }

        override fun lost(serviceName: String) {
            node?.serviceLost(serviceName)
        }
    }

    private val listener = object : BeamListener {
        override fun deviceFound(device: NearbyDevice) = _state.update { s ->
            s.copy(nearby = sortNearby(s.nearby.filterNot { it.deviceId == device.deviceId } + device))
        }

        override fun deviceLost(deviceId: String) = _state.update { s ->
            s.copy(nearby = s.nearby.filterNot { it.deviceId == deviceId })
        }

        override fun offerReceived(offer: IncomingOffer) = _state.update { s ->
            s.copy(offers = s.offers.filterNot { it.offerId == offer.offerId } + offer)
        }

        override fun offerClosed(offerId: String) = _state.update { s ->
            s.copy(offers = s.offers.filterNot { it.offerId == offerId })
        }

        override fun transferStarted(transfer: TransferInfo) = _state.update { s ->
            val row = TransferUi(
                id = transfer.transferId,
                sending = transfer.direction == Direction.SEND,
                peerName = transfer.peerName,
                fileNames = transfer.fileNames,
                fileCount = transfer.fileCount.toInt(),
                totalBytes = transfer.totalBytes.toLong(),
            )
            s.copy(transfers = TransferReducer.started(s.transfers, row))
        }

        override fun transferStatus(transferId: String, status: String, verificationCode: String?) = _state.update { s ->
            s.copy(transfers = TransferReducer.status(s.transfers, transferId, status, verificationCode))
        }

        override fun transferProgress(transferId: String, doneBytes: ULong, totalBytes: ULong) = _state.update { s ->
            s.copy(transfers = TransferReducer.progress(s.transfers, transferId, doneBytes.toLong(), totalBytes.toLong(), clock()))
        }

        override fun transferFinished(transferId: String, result: TransferResult, message: String?, savedFiles: List<String>) {
            val phase = when (result) {
                TransferResult.COMPLETED -> Phase.DONE
                TransferResult.DECLINED -> Phase.DECLINED
                TransferResult.CANCELLED -> Phase.CANCELLED
                TransferResult.FAILED -> Phase.FAILED
                TransferResult.INTERRUPTED -> Phase.INTERRUPTED
            }
            val status = when {
                phase == Phase.INTERRUPTED -> "Connection lost. It continues if the sender reconnects."
                savedFiles.isNotEmpty() -> SAVING
                else -> message
            }
            _state.update { s -> s.copy(transfers = TransferReducer.finished(s.transfers, transferId, phase, status)) }
            if (savedFiles.isEmpty()) return
            scope.launch(io) {
                val uris = runCatching { saveReceived(savedFiles) }.getOrDefault(emptyList())
                val note = if (uris.size == savedFiles.size) null else "Some files stayed in Beam's own storage"
                _state.update { s ->
                    val saved = TransferReducer.saved(s.transfers, transferId, uris)
                    s.copy(transfers = saved.map { if (it.id == transferId) it.copy(status = note) else it })
                }
            }
        }

        override fun paired(device: TrustedDevice) {
            refreshTrusted()
            showMessage("Paired with ${device.name}")
        }

        override fun advertisementChanged(advert: ServiceAdvert?) {
            platformDiscovery?.advertise(advert)
        }

        override fun bleAdvertisementChanged(advert: BleAdvertisement?) {
            radio?.advertise(advert)
        }
    }

    private fun close(node: BeamNodeInterface) {
        (node as? AutoCloseable)?.close()
    }

    companion object {
        /** Devices on the network first, then the nearest Bluetooth ones, then by name. */
        fun sortNearby(devices: List<NearbyDevice>): List<NearbyDevice> = devices.sortedWith(
            compareBy<NearbyDevice> { it.link != NearbyLink.LAN }
                .thenByDescending { it.rssi?.toInt() ?: 0 }
                .thenBy { it.name.lowercase() },
        )

        const val EVERYONE_FOR_MS = 10 * 60 * 1000L
        /** Status of a received transfer until its files are in Downloads. */
        const val SAVING = "Saving to Downloads…"

        fun describe(e: Throwable): String = when (e) {
            is BeamException.Failed -> e.reason
            else -> e.message ?: e.javaClass.simpleName
        }
    }
}
