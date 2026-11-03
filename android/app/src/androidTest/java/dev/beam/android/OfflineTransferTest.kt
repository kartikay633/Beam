package dev.beam.android

import android.os.SystemClock
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.beam.android.model.TransferUi.Phase
import dev.beam.core.FileSource
import dev.beam.core.NearbyLink
import dev.beam.core.OutgoingFile
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assume.assumeNotNull
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/** Opt-in hardware acceptance: the peer must run Beam and allow this phone. */
@RunWith(AndroidJUnit4::class)
class OfflineTransferTest {
    @Test
    fun consecutiveOfflineTransfers() = runBlocking {
        val target = InstrumentationRegistry.getArguments().getString("beamOfflinePeer")
        assumeNotNull(target)
        val app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as BeamApp
        val repository = app.repository
        repository.start()
        repeat(2) { attempt ->
            // Once P2P forms, mDNS can find the peer on that private network.
            val matches: (dev.beam.core.NearbyDevice) -> Boolean = { it.name.equals(target, true) }
            val peer = withTimeout(20_000) {
                repository.state.first { s -> s.nearby.any(matches) }.nearby.first(matches)
            }
            assumeTrue("Cold Direct acceptance needs Bluetooth-only discovery; the peer is already on LAN",
                attempt > 0 || peer.link == NearbyLink.BLUETOOTH)
            Log.i("BeamDirect", "hardware transfer ${attempt + 1}: discovered via ${peer.link}, address ${peer.address}")
            val file = File(app.cacheDir, "beam-offline-proof-${System.currentTimeMillis()}-$attempt.bin")
            file.writeBytes(ByteArray(65_536) { (it % 251).toByte() })
            val started = SystemClock.elapsedRealtime()
            val id = repository.send(peer, listOf(OutgoingFile(file.name, 0, FileSource.Path(file.path))))
            try {
                val terminal = withTimeout(110_000) {
                    repository.state.first { s -> s.transfers.any { it.id == id && it.phase !in setOf(Phase.WAITING, Phase.ACTIVE) } }
                        .transfers.first { it.id == id }
                }
                Log.i("BeamDirect", "hardware transfer ${attempt + 1}: ${terminal.phase} in ${SystemClock.elapsedRealtime() - started} ms (${terminal.status})")
                assertEquals("attempt ${attempt + 1}: ${terminal.status}", Phase.DONE, terminal.phase)
            } finally {
                repository.cancel(id)
                file.delete()
            }
            delay(3_000)
        }
    }
}
