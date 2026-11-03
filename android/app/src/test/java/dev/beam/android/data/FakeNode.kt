package dev.beam.android.data

import dev.beam.core.BeamException
import dev.beam.core.BeamListener
import dev.beam.core.BeamNodeInterface
import dev.beam.core.BleAdvertisement
import dev.beam.core.BleClientSession
import dev.beam.core.BleConnectRequest
import dev.beam.core.BleServerSession
import dev.beam.core.NearbyDevice
import dev.beam.core.OutgoingFile
import dev.beam.core.ResolvedTarget
import dev.beam.core.TransferResult
import dev.beam.core.TrustedDevice
import dev.beam.core.Visibility

/** Stands in for the Rust node: records calls, and lets tests fire listener events. */
class FakeNode(val listener: BeamListener, var mode: Visibility) : BeamNodeInterface, AutoCloseable {
    val calls = mutableListOf<String>()
    var startFailure: Exception? = null
    var closed = false
    override fun close() { closed = true }
    var trusted = mutableListOf<TrustedDevice>()
    var sendResult: () -> TransferResult = { TransferResult.COMPLETED }
    var resolveResult: () -> ResolvedTarget = { ResolvedTarget("192.168.1.20:9000", "aaaa-bbbb-cccc-dddd", "ThinkPad") }
    var pairResult: () -> TrustedDevice = { TrustedDevice("aaaa-bbbb-cccc-dddd", "ThinkPad", true) }
    val sent = mutableListOf<Pair<String, List<OutgoingFile>>>()
    var lan = emptyList<String>()
    /** What `bluetoothRoute` finds, by device id. */
    val routes = mutableMapOf<String, NearbyDevice>()
    var sendToResult: () -> TransferResult = { TransferResult.COMPLETED }

    override fun cancel(transferId: String) {
        calls += "cancel $transferId"
    }

    override fun deviceId() = "1111-2222-3333-4444"

    override fun deviceName() = "Pixel"

    override suspend fun pair(code: String): TrustedDevice {
        calls += "pair"
        return pairResult().also { trusted += it }
    }

    override suspend fun pairAt(code: String, address: String): TrustedDevice {
        calls += "pair at $address"
        return TrustedDevice("aaaa-bbbb-cccc-dddd", "ThinkPad", true).also { trusted += it }
    }

    override fun removeTrusted(deviceId: String) {
        trusted.removeAll { it.deviceId == deviceId }
    }

    override fun respondOffer(offerId: String, accept: Boolean, trust: Boolean) {
        calls += "respond $offerId $accept $trust"
    }

    override suspend fun send(transferId: String, target: String, files: List<OutgoingFile>): TransferResult {
        sent += target to files
        return sendResult()
    }

    override suspend fun resolveTarget(target: String): ResolvedTarget = resolveResult()

    override fun serviceFound(serviceName: String, txt: Map<String, String>, addresses: List<String>, port: UShort) {
        calls += "found $serviceName"
    }

    override fun serviceLost(serviceName: String) {
        calls += "lost $serviceName"
    }

    override fun setAutoAccept(deviceId: String, autoAccept: Boolean) {
        val i = trusted.indexOfFirst { it.deviceId == deviceId }
        if (i < 0) throw BeamException.Failed("not a trusted device")
        trusted[i] = trusted[i].copy(autoAccept = autoAccept)
    }

    val saveTo = mutableMapOf<String, String?>()
    override fun setSaveTo(transferId: String, folder: String?) {
        saveTo[transferId] = folder
    }

    override fun setStreams(streams: UInt) {
        calls += "streams $streams"
    }

    override fun setVisibility(visibility: Visibility) {
        mode = visibility
    }

    override fun start(): UShort {
        calls += "start"
        startFailure?.let { throw it }
        return 40000u
    }

    override fun stop() {
        calls += "stop"
    }

    override fun trustedDevices(): List<TrustedDevice> = trusted.toList()

    override fun visibility() = mode

    override fun bleAdvertisement(): BleAdvertisement? = null

    override fun bleClientSession(info: ByteArray, request: BleConnectRequest): BleClientSession = error("not used")

    override fun bleSeen(bleAddress: String, serviceData: ByteArray, localName: String?, rssi: Short) {
        calls += "ble seen $bleAddress"
    }

    override fun bleServerSession(): BleServerSession = error("not used")

    override fun bleCallAdvertisement(deviceId: String): BleAdvertisement? = null

    override fun port(): UShort? = 40000u

    override suspend fun sendTo(
        transferId: String,
        address: String,
        deviceId: String?,
        peerName: String,
        files: List<OutgoingFile>,
    ): TransferResult {
        sent += address to files
        return sendToResult()
    }

    override fun setLanAddresses(addresses: List<String>) {
        lan = addresses
    }

    override fun sharedLanAddress(request: BleConnectRequest): String? = null

    override fun bluetoothRoute(deviceId: String): NearbyDevice? = routes[deviceId]
}
