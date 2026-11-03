package dev.beam.android.radio

import org.junit.Assert.*
import org.junit.Test

class RetainedLinkTest {
    @Test fun completionKeepsTheLinkForTheNextTransfer() {
        var removed = 0
        val link = RetainedLink("group") { removed++ }
        val timers = mutableListOf<Long>()
        val first = link.acquire(timers::add)!!
        assertTrue(link.isOpen)
        first.release()
        first.release()
        assertEquals("finishing one file must not tear down Wi-Fi Direct", 0, removed)
        assertEquals(1, timers.size)
        val second = link.acquire(timers::add)!!
        link.expire(timers.first())
        assertEquals("an old idle timer removed the next transfer's group", 0, removed)
        assertTrue(link.inUse)
        second.release()
        link.expire(timers.last())
        assertEquals(1, removed)
        assertFalse(link.isOpen)
        assertNull(link.acquire(timers::add))
    }

    @Test fun overlappingLeasesKeepTheGroupUntilTheLastRelease() {
        var removed = 0
        val link = RetainedLink("group") { removed++ }
        val timers = mutableListOf<Long>()
        val a = link.acquire(timers::add)!!
        val b = link.acquire(timers::add)!!
        a.release()
        assertTrue(timers.isEmpty())
        b.release()
        assertEquals(1, timers.size)
        link.expire(timers.single())
        link.close()
        b.release()
        assertEquals(1, removed)
    }

    @Test fun stoppingClosesAnActiveLinkExactlyOnce() {
        var removed = 0
        val link = RetainedLink("group") { removed++ }
        val lease = link.acquire { fail("closed links must not schedule idle cleanup") }!!
        link.close()
        lease.release()
        link.close()
        assertEquals(1, removed)
        assertFalse(link.inUse)
    }
}
