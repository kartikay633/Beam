package dev.beam.android.data

import dev.beam.core.BeamException
import dev.beam.core.BeamListener
import dev.beam.core.BeamNodeInterface
import dev.beam.core.OutgoingFile
import dev.beam.core.TransferResult
import dev.beam.core.TrustedDevice
import dev.beam.core.Visibility

/** Stands in for the Rust node: records calls, and lets tests fire listener events. */
class FakeNode(val listener: BeamListener, var mode: Visibility) : BeamNodeInterface {
    val calls = mutableListOf<String>()
    var trusted = mutableListOf<TrustedDevice>()
    var sendResult: () -> TransferResult = { TransferResult.COMPLETED }
    var pairResult: () -> TrustedDevice = { TrustedDevice("aaaa-bbbb-cccc-dddd", "ThinkPad", true) }
    val sent = mutableListOf<Pair<String, List<OutgoingFile>>>()

    override fun cancel(transferId: String) {
        calls += "cancel $transferId"
    }

    override fun deviceId() = "1111-2222-3333-4444"

    override fun deviceName() = "Pixel"

    override suspend fun pair(code: String): TrustedDevice {
        calls += "pair"
        return pairResult().also { trusted += it }
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

    override fun setStreams(streams: UInt) {
        calls += "streams $streams"
    }

    override fun setVisibility(visibility: Visibility) {
        mode = visibility
    }

    override fun start(): UShort {
        calls += "start"
        return 40000u
    }

    override fun stop() {
        calls += "stop"
    }

    override fun trustedDevices(): List<TrustedDevice> = trusted.toList()

    override fun visibility() = mode
}
