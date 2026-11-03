package dev.beam.android.radio

import org.junit.Assert.*
import org.junit.Test

class P2pGenerationTest {
    @Test fun delayedCleanupCannotRemoveTheNextTransfersGroup() {
        val groups = P2pGeneration()
        val previous = groups.begin()
        val current = groups.begin()
        var currentGroupUp = true
        var oldChannelClosed = false
        groups.release(previous, { currentGroupUp = false }, { oldChannelClosed = true })
        assertTrue("the previous transfer removed the current group's connection", currentGroupUp)
        assertTrue(oldChannelClosed)
        groups.release(current, { currentGroupUp = false }, { fail("current group was skipped") })
        assertFalse(currentGroupUp)
    }
}
