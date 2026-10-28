package dev.beam.android.model

import dev.beam.android.model.TransferUi.Phase
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TransferReducerTest {
    private fun transfer(id: String = "t", total: Long = 10_000_000) = TransferUi(
        id = id,
        sending = true,
        peerName = "Laptop",
        fileNames = listOf("a.jpg"),
        fileCount = 1,
        totalBytes = total,
    )

    @Test
    fun newestTransferComesFirst() {
        val list = TransferReducer.started(TransferReducer.started(emptyList(), transfer("1")), transfer("2"))
        assertEquals(listOf("2", "1"), list.map { it.id })
    }

    @Test
    fun speedAndEtaComeFromProgressSamples() {
        var list = TransferReducer.started(emptyList(), transfer())
        list = TransferReducer.progress(list, "t", 0, 10_000_000, nowMs = 1_000)
        list = TransferReducer.progress(list, "t", 2_000_000, 10_000_000, nowMs = 2_000)
        val t = list.single()
        assertEquals(Phase.ACTIVE, t.phase)
        assertEquals(2_000_000.0, t.bytesPerSecond, 0.1)
        assertEquals(4.0, t.etaSeconds!!, 0.01)
        assertEquals(0.2f, t.fraction, 0.001f)
    }

    @Test
    fun samplesCloserThanHalfASecondDoNotChangeTheSpeed() {
        var list = TransferReducer.started(emptyList(), transfer())
        list = TransferReducer.progress(list, "t", 0, 10_000_000, nowMs = 1_000)
        list = TransferReducer.progress(list, "t", 5_000_000, 10_000_000, nowMs = 1_100)
        assertEquals(0.0, list.single().bytesPerSecond, 0.0)
        assertNull(list.single().etaSeconds)
    }

    @Test
    fun reconnectingAndResumingAreShown() {
        var list = TransferReducer.started(emptyList(), transfer())
        list = TransferReducer.status(list, "t", "Connection lost, reconnecting (attempt 1)…", null)
        assertEquals(Phase.INTERRUPTED, list.single().phase)
        list = TransferReducer.status(list, "t", "Sending…", null)
        assertEquals(Phase.ACTIVE, list.single().phase)
    }

    @Test
    fun finishingCompletesTheBarAndClearingRemovesIt() {
        var list = TransferReducer.started(emptyList(), transfer())
        list = TransferReducer.finished(list, "t", Phase.DONE, null)
        assertEquals(1f, list.single().fraction)
        assertTrue(list.single().finished)
        assertTrue(TransferReducer.clearFinished(list).isEmpty())
    }

    @Test
    fun aResumedTransferKeepsItsRow() {
        var list = TransferReducer.started(emptyList(), transfer())
        list = TransferReducer.progress(list, "t", 4_000_000, 10_000_000, nowMs = 1_000)
        list = TransferReducer.started(list, transfer())
        assertEquals(1, list.size)
        assertEquals(4_000_000, list.single().doneBytes)
    }

    @Test
    fun onlyRecentFinishedTransfersAreKept() {
        var list = emptyList<TransferUi>()
        repeat(TransferReducer.MAX_FINISHED + 5) { i ->
            list = TransferReducer.started(list, transfer("$i"))
            list = TransferReducer.finished(list, "$i", Phase.DONE, null)
        }
        assertEquals(TransferReducer.MAX_FINISHED, list.size)
        assertEquals("${TransferReducer.MAX_FINISHED + 4}", list.first().id)
    }
}
