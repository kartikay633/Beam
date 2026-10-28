package dev.beam.android.radio

import android.annotation.SuppressLint
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.content.Context
import android.os.Build
import dev.beam.android.data.OfflineLink
import dev.beam.core.BeamNodeInterface
import dev.beam.core.BleConnectRequest
import dev.beam.core.BleConnectResponse
import dev.beam.core.BleLink
import dev.beam.core.BleUuids
import dev.beam.core.HotspotCredentials
import dev.beam.core.NearbyDevice
import java.util.UUID
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withTimeout

/**
 * The sender's side of the Bluetooth handshake: connect to the receiver's
 * Beam GATT service, read `INFO`, write a `CONNECT_REQUEST` offering this
 * phone's hotspot, and wait for the `CONNECT_RESPONSE` naming the address
 * to send to.
 */
@SuppressLint("MissingPermission") // BleRadio checks RadioPermissions before connecting.
internal class BleConnector(
    private val context: Context,
    private val node: BeamNodeInterface,
    private val hotspot: LocalHotspot,
    uuids: BleUuids,
) {
    private val serviceUuid = UUID.fromString(uuids.service)
    private val infoUuid = UUID.fromString(uuids.info)
    private val requestUuid = UUID.fromString(uuids.connectRequest)
    private val responseUuid = UUID.fromString(uuids.connectResponse)
    private val preferredMtu = uuids.preferredMtu.toInt()

    suspend fun connect(device: NearbyDevice, status: (String) -> Unit): OfflineLink = coroutineScope {
        val address = checkNotNull(device.bleAddress) { "${device.name} has no Bluetooth address" }
        val remote = context.getSystemService(BluetoothManager::class.java).adapter.getRemoteDevice(address)
        status("Starting a private Wi-Fi link…")
        // The hotspot takes a moment; start it while Bluetooth connects.
        val hosting = async { hotspot.acquire() }
        var acquired = false
        var keepHotspot = false
        try {
            status("Connecting to ${device.name} over Bluetooth…")
            val response = Gatt(remote).use { gatt ->
                gatt.open()
                val info = gatt.read(infoUuid)
                gatt.enableNotifications(responseUuid)
                val spot = hosting.await()
                acquired = true
                val session = node.bleClientSession(
                    info,
                    BleConnectRequest(
                        links = listOf(BleLink.LAN, BleLink.HOTSPOT),
                        lan = LocalHotspot.ipv4Addresses().map { it.second }.filter { it != spot.address },
                        hotspot = HotspotCredentials(spot.ssid, spot.passphrase),
                        capabilities = 0u,
                    ),
                )
                for (fragment in session.requestFragments(gatt.mtu.toUInt())) gatt.write(requestUuid, fragment)
                status("Waiting for ${device.name} to join…")
                // The laptop joins the hotspot before it answers.
                withTimeout(RESPONSE_TIMEOUT_MS) {
                    var whole: BleConnectResponse? = null
                    while (whole == null) whole = session.receive(gatt.notifications.receive())
                    whole
                }
            }
            val link = checkNotNull(response.link) { response.reason ?: "${device.name} declined" }
            check(response.hotspot == null) { "${device.name} offered its own hotspot, which phones can't join yet" }
            val target = checkNotNull(response.address) { "${device.name} didn't say where to connect" }
            keepHotspot = link == BleLink.HOTSPOT
            OfflineLink(
                address = target,
                deviceId = device.deviceId,
                release = { if (keepHotspot) hotspot.release() },
            )
        } catch (e: Throwable) {
            hosting.cancel()
            throw e
        } finally {
            // Not needed when the LAN was chosen, or the handshake failed.
            if (acquired && !keepHotspot) hotspot.release()
        }
    }

    /** One GATT client connection, one operation at a time. */
    private inner class Gatt(private val device: BluetoothDevice) : AutoCloseable {
        private var gatt: BluetoothGatt? = null
        private var pending: CompletableDeferred<Any?>? = null
        private val connected = CompletableDeferred<Unit>()
        val notifications = Channel<ByteArray>(Channel.UNLIMITED)
        var mtu = 23
            private set

        private val callback = object : BluetoothGattCallback() {
            override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
                if (newState == BluetoothProfile.STATE_CONNECTED && status == BluetoothGatt.GATT_SUCCESS) {
                    connected.complete(Unit)
                } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                    val error = IllegalStateException("Bluetooth connection lost (status $status)")
                    connected.completeExceptionally(error)
                    pending?.completeExceptionally(error)
                    notifications.close(error)
                }
            }

            override fun onMtuChanged(g: BluetoothGatt, mtu: Int, status: Int) {
                if (status == BluetoothGatt.GATT_SUCCESS) this@Gatt.mtu = mtu
                pending?.complete(Unit)
            }

            override fun onServicesDiscovered(g: BluetoothGatt, status: Int) = finish(status, Unit)

            @Deprecated("Before Android 13")
            override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
                @Suppress("DEPRECATION")
                finish(status, c.value)
            }

            override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray, status: Int) =
                finish(status, value)

            override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) = finish(status, Unit)

            override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) = finish(status, Unit)

            @Deprecated("Before Android 13")
            override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
                @Suppress("DEPRECATION")
                c.value?.let { notifications.trySend(it.copyOf()) }
            }

            override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, value: ByteArray) {
                notifications.trySend(value)
            }
        }

        private fun finish(status: Int, value: Any?) {
            val waiting = pending ?: return
            if (status == BluetoothGatt.GATT_SUCCESS) waiting.complete(value)
            else waiting.completeExceptionally(IllegalStateException("Bluetooth operation failed (status $status)"))
        }

        private suspend fun <T> op(timeoutMs: Long = 10_000, start: (BluetoothGatt) -> Boolean): T {
            val g = checkNotNull(gatt)
            val done = CompletableDeferred<Any?>()
            pending = done
            check(start(g)) { "Bluetooth is busy" }
            @Suppress("UNCHECKED_CAST")
            return withTimeout(timeoutMs) { done.await() } as T
        }

        suspend fun open() {
            gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
            withTimeout(15_000) { connected.await() }
            runCatching { op<Unit>(5_000) { it.requestMtu(preferredMtu) } }
            op<Unit>(15_000) { it.discoverServices() }
            checkNotNull(gatt?.getService(serviceUuid)) { "${device.address} isn't running Beam" }
        }

        private fun characteristic(uuid: UUID): BluetoothGattCharacteristic =
            checkNotNull(gatt?.getService(serviceUuid)?.getCharacteristic(uuid)) { "Beam service is incomplete" }

        suspend fun read(uuid: UUID): ByteArray = op { it.readCharacteristic(characteristic(uuid)) }

        @Suppress("DEPRECATION") // The value-taking overloads only exist from Android 13.
        suspend fun write(uuid: UUID, value: ByteArray) {
            val c = characteristic(uuid)
            op<Unit> { g ->
                if (Build.VERSION.SDK_INT >= 33) {
                    g.writeCharacteristic(c, value, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT) == BluetoothStatusCodes.SUCCESS
                } else {
                    c.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
                    c.value = value
                    g.writeCharacteristic(c)
                }
            }
        }

        @Suppress("DEPRECATION")
        suspend fun enableNotifications(uuid: UUID) {
            val c = characteristic(uuid)
            val g = checkNotNull(gatt)
            check(g.setCharacteristicNotification(c, true)) { "can't subscribe to Beam's response" }
            val cccd = checkNotNull(c.getDescriptor(BleRadio.CCCD)) { "Beam service is incomplete" }
            val enable = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
            op<Unit> {
                if (Build.VERSION.SDK_INT >= 33) {
                    it.writeDescriptor(cccd, enable) == BluetoothStatusCodes.SUCCESS
                } else {
                    cccd.value = enable
                    it.writeDescriptor(cccd)
                }
            }
        }

        override fun close() {
            gatt?.let {
                runCatching { it.disconnect() }
                runCatching { it.close() }
            }
            gatt = null
        }
    }

    private companion object {
        /** Long enough for the laptop to join the hotspot and answer. */
        const val RESPONSE_TIMEOUT_MS = 60_000L
    }
}
