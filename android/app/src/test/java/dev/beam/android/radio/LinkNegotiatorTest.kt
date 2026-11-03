package dev.beam.android.radio

import dev.beam.core.BleConnectRequest
import dev.beam.core.BleDirectPeer
import dev.beam.core.BleLink
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

class LinkNegotiatorTest {
    private fun request() = BleConnectRequest(
        listOf(BleLink.LAN, BleLink.WIFI_DIRECT, BleLink.HOTSPOT),
        listOf("192.168.1.4:9000"), null, 0u, null, BleDirectPeer("Laptop", null, null),
    )

    @Test fun failedDirectFallsBackToHotspotAndReleasesIt() = runTest {
        var released = false
        val negotiator = LinkNegotiator({ _, _ -> null }, { null }, {
            HotspotRoute("Beam", "secret", "192.168.43.1") { released = true }
        }, 47813)
        val link = negotiator.negotiate(request(), 40000)
        assertEquals(BleLink.HOTSPOT, link.response.link)
        assertEquals("192.168.43.1:40000", link.response.address)
        assertEquals("secret", link.response.hotspot?.passphrase)
        assertNull(link.sendTarget)
        link.release()
        assertTrue(released)
    }

    @Test fun outgoingCallAlsoFallsBackAndWaitsForTheLaptopsHello() = runTest {
        val negotiator = LinkNegotiator({ _, _ -> null }, { null }, {
            HotspotRoute("Beam", "secret", null) {}
        }, 47813)
        val link = negotiator.negotiate(request(), 40000, sendPort = 9000)
        assertEquals(BleLink.HOTSPOT, link.response.link)
        assertEquals("0.0.0.0:47813", link.response.address)
        assertNull(link.sendTarget)
    }

    @Test fun reachableLanIsPreferredAndUsesTheProbedRoute() = runTest {
        val negotiator = LinkNegotiator({ _, _ -> LanRoute("10.20.1.9", "10.20.2.4") },
            { error("must not start Direct") }, { error("must not start hotspot") }, 47813)
        val link = negotiator.negotiate(request(), 40000, sendPort = 9000)
        assertEquals(BleLink.LAN, link.response.link)
        assertEquals("10.20.2.4:9000", link.sendTarget)
    }

    @Test fun blockedLanUsesDirectAndReturnsThePhonesReceivingPort() = runTest {
        var released = false
        val negotiator = LinkNegotiator({ _, _ -> null }, {
            DirectRoute("192.168.137.2", "192.168.137.1") { released = true }
        }, { error("Direct worked") }, 47813)
        val link = negotiator.negotiate(request(), 40000)
        assertEquals(BleLink.WIFI_DIRECT, link.response.link)
        assertEquals("192.168.137.2:40000", link.response.address)
        link.release()
        assertTrue(released)
    }

    @Test fun reachableRetainedDirectKeepsItsLeaseInBothDirections() = runTest {
        for (sendPort in listOf(null, 9000)) {
            var released = 0
            val negotiator = LinkNegotiator({ _, _ ->
                LanRoute("192.168.137.2", "192.168.137.1") { released++ }
            }, { error("the established route is already leased") }, { error("must not start hotspot") }, 47813)
            val link = negotiator.negotiate(request(), 40000, sendPort)
            assertEquals(BleLink.WIFI_DIRECT, link.response.link)
            assertEquals(if (sendPort == null) "192.168.137.2:40000" else "192.168.137.2", link.response.address)
            assertEquals(if (sendPort == null) null else "192.168.137.1:9000", link.sendTarget)
            assertEquals(0, released)
            link.release()
            assertEquals(1, released)
        }
    }

    @Test fun cancellationDuringDirectNeverStartsAHotspot() = runTest {
        val negotiator = LinkNegotiator({ _, _ -> null }, { throw CancellationException("cancelled") },
            { error("must not start hotspot after cancellation") }, 47813)
        try {
            negotiator.negotiate(request(), 40000)
            fail("cancellation must propagate")
        } catch (e: CancellationException) {
            assertEquals("cancelled", e.message)
        }
    }
}
