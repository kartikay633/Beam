package dev.beam.android.radio

import org.junit.Assert.*
import org.junit.Test

class LinkLeaseTest {
    @Test fun aLateAcquisitionIsReleasedAfterCancellation() {
        val lease = LinkLease()
        lease.close()
        var released = 0
        assertFalse(lease.attach { released++ })
        assertEquals(1, released)
    }

    @Test fun aTransferredLinkIsReleasedExactlyOnce() {
        val lease = LinkLease()
        var released = 0
        assertTrue(lease.attach { released++ })
        lease.close()
        lease.close()
        assertEquals(1, released)
    }
}
