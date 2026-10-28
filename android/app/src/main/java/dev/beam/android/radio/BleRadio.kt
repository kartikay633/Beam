package dev.beam.android.radio

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattServer
import android.bluetooth.BluetoothGattServerCallback
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.ParcelUuid
import android.util.Log
import androidx.core.content.ContextCompat
import dev.beam.android.data.OfflineLink
import dev.beam.android.data.OfflineRadio
import dev.beam.core.BeamNodeInterface
import dev.beam.core.BleAdvertisement
import dev.beam.core.BleConnectRequest
import dev.beam.core.BleConnectResponse
import dev.beam.core.BleLink
import dev.beam.core.BleServerSession
import dev.beam.core.HotspotCredentials
import dev.beam.core.NearbyDevice
import dev.beam.core.DeviceKind
import dev.beam.core.bleSharedLan
import dev.beam.core.bleUuids
import dev.beam.core.parseBleHello
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetSocketAddress
import java.net.SocketTimeoutException
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Beam over Bluetooth LE (SPEC.md 4.2): advertises this phone, scans for
 * other Beam devices, and hosts the GATT service a sender talks to. The
 * bytes all come from the Rust node; this class only moves them through the
 * radio.
 *
 * As a receiver, it answers a `CONNECT_REQUEST` with the LAN when both are
 * on one network, or else starts the phone's hotspot for the sender to join.
 *
 * Sending to a laptop works the other way round, because Windows can't
 * reliably host a GATT service: the phone *calls* the laptop (a second
 * advertisement naming it), the laptop connects to this phone's GATT
 * service, gets the hotspot, joins it, and says hello over UDP with the
 * port it receives on.
 */
@SuppressLint("MissingPermission") // Every entry point checks RadioPermissions first.
class BleRadio(
    private val context: Context,
    private val hotspot: LocalHotspot,
    private val scope: CoroutineScope,
    /** Whether a received transfer is under way (keeps the hotspot up). */
    private val receiving: () -> Boolean,
) : OfflineRadio {
    private val manager = context.getSystemService(BluetoothManager::class.java)
    private val adapter: BluetoothAdapter? get() = manager?.adapter
    private val uuids = bleUuids()
    private val serviceUuid = UUID.fromString(uuids.service)
    private val infoUuid = UUID.fromString(uuids.info)
    private val requestUuid = UUID.fromString(uuids.connectRequest)
    private val responseUuid = UUID.fromString(uuids.connectResponse)

    private val lock = Any()
    @Volatile private var node: BeamNodeInterface? = null
    private var advert: BleAdvertisement? = null
    private var advertiseCallback: AdvertiseCallback? = null
    private var scanCallback: ScanCallback? = null
    private var server: BluetoothGattServer? = null
    private var stateReceiver: BroadcastReceiver? = null

    private class Peer(var session: BleServerSession?, var mtu: Int = 23) {
        var notificationSent: CompletableDeferred<Unit>? = null
    }

    private val peers = ConcurrentHashMap<String, Peer>()

    /** An outgoing call: waiting for the called device to connect to us. */
    private class Call(val hotspot: LocalHotspot.Info) {
        /** Its LAN `ip:port` when we share a network, or null to wait for its hello. */
        val answered = CompletableDeferred<String?>()
    }

    @Volatile private var call: Call? = null
    private var callCallback: AdvertiseCallback? = null

    override val available: Boolean
        get() = RadioPermissions.hasBluetoothLe(context) && RadioPermissions.granted(context) && adapter?.isEnabled == true

    override fun start(node: BeamNodeInterface) {
        synchronized(lock) {
            this.node = node
            if (stateReceiver == null) {
                // Bluetooth switched on or off while Beam runs.
                val receiver = object : BroadcastReceiver() {
                    override fun onReceive(c: Context, intent: Intent) = refresh()
                }
                stateReceiver = receiver
                ContextCompat.registerReceiver(
                    context,
                    receiver,
                    IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED),
                    ContextCompat.RECEIVER_NOT_EXPORTED,
                )
            }
        }
        refresh()
    }

    override fun stop() = synchronized(lock) {
        node = null
        stopRadio()
        stateReceiver?.let { runCatching { context.unregisterReceiver(it) } }
        stateReceiver = null
    }

    /** Starts or stops the radio to match permissions and the Bluetooth switch. */
    override fun refresh() = synchronized(lock) {
        stopRadio()
        if (node == null || !available) return
        openServer()
        startScan()
        applyAdvert()
    }

    override fun advertise(advert: BleAdvertisement?) = synchronized(lock) {
        this.advert = advert
        if (node != null && available) applyAdvert()
    }

    override suspend fun connect(device: NearbyDevice, status: (String) -> Unit): OfflineLink {
        val current = checkNotNull(node) { "Beam isn't running" }
        check(available) { "Turn on Bluetooth to reach ${device.name} without Wi-Fi" }
        // Phones host a GATT service; laptops are called instead.
        return if (device.kind == DeviceKind.PHONE) {
            BleConnector(context, current, hotspot, uuids).connect(device, status)
        } else {
            callAndWait(current, device, status)
        }
    }

    private suspend fun callAndWait(node: BeamNodeInterface, device: NearbyDevice, status: (String) -> Unit): OfflineLink {
        val advert = checkNotNull(node.bleCallAdvertisement(device.deviceId)) { "${device.name} is out of Bluetooth range" }
        check(call == null) { "Another transfer is setting up; try again in a moment" }
        status("Starting a private Wi-Fi link…")
        val spot = hotspot.acquire()
        var keep = false
        val socket = DatagramSocket(null).apply {
            reuseAddress = true
            soTimeout = 1_000
            bind(InetSocketAddress(uuids.helloPort.toInt()))
        }
        val pending = Call(spot)
        call = pending
        startCallAdvertising(advert)
        try {
            status("Asking ${device.name} to connect…")
            val answer = withTimeoutOrNull(CALL_TIMEOUT_MS) { pending.answered.await() }
            if (!pending.answered.isCompleted) error("${device.name} didn't answer. Is Beam open on it, with Bluetooth on?")
            stopCallAdvertising()
            if (answer != null) return OfflineLink(answer, device.deviceId) {}
            status("Waiting for ${device.name} to join…")
            val (address, deviceId) = withTimeoutOrNull(JOIN_TIMEOUT_MS) { awaitHello(socket) }
                ?: error("${device.name} couldn't join this phone's Wi-Fi")
            keep = true
            return OfflineLink(address, deviceId) { hotspot.release() }
        } finally {
            call = null
            stopCallAdvertising()
            socket.close()
            if (!keep) hotspot.release()
        }
    }

    /** The first hello on our hotspot: (`ip:port` to send to, its device id). */
    private suspend fun awaitHello(socket: DatagramSocket): Pair<String, String> = withContext(Dispatchers.IO) {
        val buffer = ByteArray(64)
        while (true) {
            ensureActive()
            val packet = DatagramPacket(buffer, buffer.size)
            try {
                socket.receive(packet)
            } catch (_: SocketTimeoutException) {
                continue
            }
            val hello = parseBleHello(packet.data.copyOf(packet.length)) ?: continue
            val ip = packet.address.hostAddress ?: continue
            return@withContext "$ip:${hello.port}" to hello.deviceId
        }
        @Suppress("UNREACHABLE_CODE")
        error("unreachable")
    }

    private fun startCallAdvertising(advert: BleAdvertisement): Unit = synchronized(lock) {
        val le = adapter?.bluetoothLeAdvertiser ?: return
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_HIGH)
            .setConnectable(true)
            .setTimeout(0)
            .build()
        val data = AdvertiseData.Builder()
            .addServiceData(ParcelUuid(serviceUuid), advert.serviceData)
            .setIncludeDeviceName(false)
            .build()
        val callback = object : AdvertiseCallback() {
            override fun onStartFailure(errorCode: Int) {
                Log.w(TAG, "call advertising failed: $errorCode")
            }
        }
        callCallback = callback
        runCatching { le.startAdvertising(settings, data, callback) }
        Unit
    }

    private fun stopCallAdvertising(): Unit = synchronized(lock) {
        callCallback?.let { runCatching { adapter?.bluetoothLeAdvertiser?.stopAdvertising(it) } }
        callCallback = null
    }

    private fun stopRadio() {
        val le = adapter?.bluetoothLeAdvertiser
        advertiseCallback?.let { runCatching { le?.stopAdvertising(it) } }
        advertiseCallback = null
        scanCallback?.let { runCatching { adapter?.bluetoothLeScanner?.stopScan(it) } }
        scanCallback = null
        server?.let { runCatching { it.close() } }
        server = null
        peers.clear()
    }

    // ---------------------------------------------------------- advertising

    private fun applyAdvert() {
        val le = adapter?.bluetoothLeAdvertiser ?: return
        advertiseCallback?.let { runCatching { le.stopAdvertising(it) } }
        advertiseCallback = null
        val current = advert ?: return
        startAdvertising(le, current, withName = current.localName != null)
    }

    private fun startAdvertising(le: android.bluetooth.le.BluetoothLeAdvertiser, advert: BleAdvertisement, withName: Boolean) {
        val settings = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_BALANCED)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
            .setConnectable(true)
            .setTimeout(0)
            .build()
        // 3 bytes of flags + 28 of service data: exactly the 31 legacy
        // advertising allows. The name goes in the scan response.
        val data = AdvertiseData.Builder()
            .addServiceData(ParcelUuid(serviceUuid), advert.serviceData)
            .setIncludeDeviceName(false)
            .setIncludeTxPowerLevel(false)
            .build()
        val response = AdvertiseData.Builder().setIncludeDeviceName(withName).build()
        val callback = object : AdvertiseCallback() {
            override fun onStartFailure(errorCode: Int) {
                Log.w(TAG, "advertising failed: $errorCode")
                // A long Bluetooth name overflows the scan response: go without it.
                if (errorCode == ADVERTISE_FAILED_DATA_TOO_LARGE && withName) {
                    synchronized(lock) {
                        if (advertiseCallback === this) startAdvertising(le, advert, withName = false)
                    }
                }
            }
        }
        advertiseCallback = callback
        runCatching { le.startAdvertising(settings, data, response, callback) }
            .onFailure { Log.w(TAG, "advertising failed", it) }
    }

    // ---------------------------------------------------------- scanning

    private fun startScan() {
        val scanner = adapter?.bluetoothLeScanner ?: return
        val filter = ScanFilter.Builder()
            .setServiceData(ParcelUuid(serviceUuid), ByteArray(0), ByteArray(0))
            .build()
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_BALANCED)
            .setCallbackType(ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
            .build()
        val callback = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) = report(result)

            override fun onBatchScanResults(results: MutableList<ScanResult>) = results.forEach(::report)

            override fun onScanFailed(errorCode: Int) {
                Log.w(TAG, "scan failed: $errorCode")
            }
        }
        scanCallback = callback
        runCatching { scanner.startScan(listOf(filter), settings, callback) }
            .onFailure { Log.w(TAG, "scan failed", it) }
    }

    private fun report(result: ScanResult) {
        val record = result.scanRecord ?: return
        val data = record.getServiceData(ParcelUuid(serviceUuid)) ?: return
        node?.bleSeen(result.device.address, data, record.deviceName, result.rssi.toShort())
    }

    // ---------------------------------------------------------- GATT server

    private fun openServer() {
        val gatt = manager?.openGattServer(context, serverCallback) ?: return
        val service = BluetoothGattService(serviceUuid, BluetoothGattService.SERVICE_TYPE_PRIMARY)
        service.addCharacteristic(
            BluetoothGattCharacteristic(
                infoUuid,
                BluetoothGattCharacteristic.PROPERTY_READ,
                BluetoothGattCharacteristic.PERMISSION_READ,
            ),
        )
        service.addCharacteristic(
            BluetoothGattCharacteristic(
                requestUuid,
                BluetoothGattCharacteristic.PROPERTY_WRITE,
                BluetoothGattCharacteristic.PERMISSION_WRITE,
            ),
        )
        val response = BluetoothGattCharacteristic(
            responseUuid,
            BluetoothGattCharacteristic.PROPERTY_NOTIFY,
            BluetoothGattCharacteristic.PERMISSION_READ,
        )
        response.addDescriptor(
            BluetoothGattDescriptor(
                CCCD,
                BluetoothGattDescriptor.PERMISSION_READ or BluetoothGattDescriptor.PERMISSION_WRITE,
            ),
        )
        service.addCharacteristic(response)
        gatt.addService(service)
        server = gatt
    }

    private val serverCallback = object : BluetoothGattServerCallback() {
        override fun onConnectionStateChange(device: BluetoothDevice, status: Int, newState: Int) {
            if (newState == BluetoothProfile.STATE_DISCONNECTED) peers.remove(device.address)
        }

        override fun onMtuChanged(device: BluetoothDevice, mtu: Int) {
            peers.getOrPut(device.address) { Peer(null) }.mtu = mtu
        }

        override fun onCharacteristicReadRequest(
            device: BluetoothDevice,
            requestId: Int,
            offset: Int,
            characteristic: BluetoothGattCharacteristic,
        ) {
            val gatt = server ?: return
            val current = node
            if (characteristic.uuid != infoUuid || current == null) {
                gatt.sendResponse(device, requestId, BluetoothGatt.GATT_READ_NOT_PERMITTED, offset, null)
                return
            }
            val peer = peers.getOrPut(device.address) { Peer(null) }
            // A fresh read starts a fresh handshake (and ephemeral key).
            if (offset == 0 || peer.session == null) peer.session = current.bleServerSession()
            val info = peer.session!!.info()
            if (offset > info.size) {
                gatt.sendResponse(device, requestId, BluetoothGatt.GATT_INVALID_OFFSET, offset, null)
                return
            }
            gatt.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, info.copyOfRange(offset, info.size))
        }

        override fun onCharacteristicWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            characteristic: BluetoothGattCharacteristic,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray,
        ) {
            val gatt = server ?: return
            val session = peers[device.address]?.session
            val ok = characteristic.uuid == requestUuid && session != null
            if (responseNeeded) {
                gatt.sendResponse(device, requestId, if (ok) BluetoothGatt.GATT_SUCCESS else BluetoothGatt.GATT_WRITE_NOT_PERMITTED, offset, null)
            }
            if (!ok) return
            val request = runCatching { session!!.receive(value) }
                .onFailure { Log.w(TAG, "bad connect request", it) }
                .getOrNull() ?: return
            scope.launch { answer(device, session!!, request) }
        }

        override fun onDescriptorWriteRequest(
            device: BluetoothDevice,
            requestId: Int,
            descriptor: BluetoothGattDescriptor,
            preparedWrite: Boolean,
            responseNeeded: Boolean,
            offset: Int,
            value: ByteArray,
        ) {
            if (responseNeeded) server?.sendResponse(device, requestId, BluetoothGatt.GATT_SUCCESS, offset, null)
        }

        override fun onNotificationSent(device: BluetoothDevice, status: Int) {
            peers[device.address]?.notificationSent?.complete(Unit)
        }
    }

    /** Chooses a link for the sender and notifies it. */
    private suspend fun answer(device: BluetoothDevice, session: BleServerSession, request: BleConnectRequest) {
        val current = node ?: return
        val response = runCatching { chooseLink(current, request) }.getOrElse {
            BleConnectResponse(null, null, null, it.message ?: "couldn't set up a link")
        }
        val peer = peers[device.address] ?: return
        val fragments = runCatching { session.respond(response, peer.mtu.toUInt()) }.getOrElse { return }
        val characteristic = server?.getService(serviceUuid)?.getCharacteristic(responseUuid) ?: return
        for (fragment in fragments) {
            val sent = CompletableDeferred<Unit>()
            peer.notificationSent = sent
            notify(device, characteristic, fragment)
            if (withTimeoutOrNull(5_000) { sent.await() } == null) {
                Log.w(TAG, "notification to ${device.address} timed out")
                return
            }
        }
    }

    private suspend fun chooseLink(node: BeamNodeInterface, request: BleConnectRequest): BleConnectResponse {
        val pending = call
        val calledPort = request.port
        if (calledPort != null && pending != null) return answerCall(pending, request, calledPort.toInt())
        node.sharedLanAddress(request)?.let { return BleConnectResponse(BleLink.LAN, it, null, null) }
        if (BleLink.HOTSPOT !in request.links) {
            return BleConnectResponse(null, null, null, "no common way to connect")
        }
        if (request.hotspot != null) {
            // Another phone offering its hotspot: joining one needs the
            // user's approval on every connection, so not yet.
            return BleConnectResponse(null, null, null, "phone-to-phone transfers without Wi-Fi aren't supported yet")
        }
        val port = checkNotNull(node.port()) { "Beam isn't receiving" }
        val info = hotspot.acquire()
        holdHotspotWhileReceiving()
        // 0.0.0.0 when the hotspot's address couldn't be found: the other
        // device then tries the hotspot's gateway, which is this phone.
        return BleConnectResponse(
            BleLink.HOTSPOT,
            "${info.address ?: "0.0.0.0"}:$port",
            HotspotCredentials(info.ssid, info.passphrase),
            null,
        )
    }

    /** The device we called connected: tell it how to reach us. */
    private fun answerCall(pending: Call, request: BleConnectRequest, port: Int): BleConnectResponse {
        val ours = LocalHotspot.ipv4Addresses().map { it.second }.filter { it != pending.hotspot.address }
        // On one network already: send to its LAN address.
        bleSharedLan(ours, request.lan)?.let { theirs ->
            pending.answered.complete("$theirs:$port")
            return BleConnectResponse(BleLink.LAN, null, null, null)
        }
        pending.answered.complete(null)
        return BleConnectResponse(
            BleLink.HOTSPOT,
            "${pending.hotspot.address ?: "0.0.0.0"}:${uuids.helloPort}",
            HotspotCredentials(pending.hotspot.ssid, pending.hotspot.passphrase),
            null,
        )
    }

    /** Keeps the hotspot for the transfer that should follow, then lets it go. */
    private fun holdHotspotWhileReceiving() = scope.launch {
        // Give the sender time to join and start.
        delay(90_000)
        while (receiving()) delay(5_000)
        hotspot.release()
    }

    @Suppress("DEPRECATION") // The value-taking overload only exists from Android 13.
    private fun notify(device: BluetoothDevice, characteristic: BluetoothGattCharacteristic, value: ByteArray) {
        val gatt = server ?: return
        if (Build.VERSION.SDK_INT >= 33) {
            gatt.notifyCharacteristicChanged(device, characteristic, false, value)
        } else {
            characteristic.value = value
            gatt.notifyCharacteristicChanged(device, characteristic, false)
        }
    }

    companion object {
        const val TAG = "BeamBle"
        /** How long a called laptop has to notice and connect. */
        const val CALL_TIMEOUT_MS = 45_000L
        /** How long it has to join the hotspot and say hello. */
        const val JOIN_TIMEOUT_MS = 60_000L
        val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
    }
}
