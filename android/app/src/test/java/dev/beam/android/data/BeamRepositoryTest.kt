package dev.beam.android.data

import dev.beam.android.model.TransferUi.Phase
import dev.beam.core.BeamException
import dev.beam.core.Direction
import dev.beam.core.FileSource
import dev.beam.core.IncomingOffer
import dev.beam.core.NearbyDevice
import dev.beam.core.DeviceKind
import dev.beam.core.OfferFile
import dev.beam.core.OutgoingFile
import dev.beam.core.ServiceAdvert
import dev.beam.core.TransferInfo
import dev.beam.core.TransferResult
import dev.beam.core.Visibility
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class BeamRepositoryTest {
    private val dispatcher = StandardTestDispatcher()
    private val settings = MemorySettings()
    private val saved = mutableListOf<List<String>>()
    private val discovery = FakeDiscovery()
    private lateinit var node: FakeNode

    private fun TestScope.repository(choice: DiscoveryChoice = DiscoveryChoice.ANDROID_NSD): BeamRepository {
        settings.discovery = choice
        return BeamRepository(
            // Not backgroundScope: advanceUntilIdle doesn't run work launched there.
            scope = this,
            settings = settings,
            createNode = { s, listener -> FakeNode(listener, s.visibility).also { node = it } },
            platformDiscovery = discovery,
            saveReceived = { paths ->
                saved += paths
                paths.map { "content://media/downloads/${it.substringAfterLast('/')}" }
            },
            clock = { testScheduler.currentTime },
            io = dispatcher,
        )
    }

    private val offer = IncomingOffer(
        offerId = "offer-1",
        fromName = "ThinkPad",
        fromId = "aaaa-bbbb-cccc-dddd",
        trusted = false,
        verificationCode = "482193",
        files = listOf(OfferFile("a.jpg", 1000u)),
        totalBytes = 1000u,
        timeoutSecs = 60u,
    )

    @Test
    fun startingCreatesTheNodeOnceAndStartsNsd() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        repo.start()
        assertEquals(listOf("streams 4", "start"), node.calls)
        assertTrue(discovery.started)
        val state = repo.state.value
        assertTrue(state.running)
        assertEquals("1111-2222-3333-4444", state.deviceId)
        assertEquals(Visibility.EVERYONE, state.visibility)
        assertEquals(BeamRepository.EVERYONE_FOR_MS, state.everyoneUntilMs)
    }

    @Test
    fun builtInDiscoveryLeavesNsdAlone() = runTest(dispatcher) {
        repository(DiscoveryChoice.BUILT_IN).start()
        assertFalse(discovery.started)
    }

    @Test
    fun nsdResultsGoToTheNodeAndAdvertsToNsd() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        discovery.sink!!.found("beam-1a2b", mapOf("v" to "1"), listOf("192.168.1.20"), 40000)
        discovery.sink!!.lost("beam-1a2b")
        assertEquals(listOf("found beam-1a2b", "lost beam-1a2b"), node.calls.drop(2))
        val advert = ServiceAdvert("beam-1111", 40000u, mapOf("vis" to "everyone"))
        node.listener.advertisementChanged(advert)
        assertEquals(advert, discovery.adverts.last())
    }

    @Test
    fun everyoneFallsBackToTrustedOnlyAfterTenMinutes() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        node.mode = Visibility.TRUSTED_ONLY // what core does on its own
        advanceTimeBy(BeamRepository.EVERYONE_FOR_MS + 2_000)
        assertEquals(Visibility.TRUSTED_ONLY, repo.state.value.visibility)
        assertNull(repo.state.value.everyoneUntilMs)
        assertEquals(Visibility.TRUSTED_ONLY, settings.visibility)
    }

    @Test
    fun offersAreShownUntilAnswered() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        node.listener.offerReceived(offer)
        assertEquals(listOf(offer), repo.state.value.offers)
        repo.respond("offer-1", accept = true, trust = true)
        assertTrue(repo.state.value.offers.isEmpty())
        assertTrue("respond offer-1 true true" in node.calls)
    }

    @Test
    fun anOfferThatTimesOutDisappears() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        node.listener.offerReceived(offer)
        node.listener.offerClosed("offer-1")
        assertTrue(repo.state.value.offers.isEmpty())
    }

    @Test
    fun receivedFilesAreMovedToDownloads() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        node.listener.transferStarted(TransferInfo("t1", Direction.RECEIVE, "ThinkPad", listOf("a.jpg"), 1u, 1000u))
        node.listener.transferProgress("t1", 500u, 1000u)
        assertEquals(Phase.ACTIVE, repo.state.value.transfers.single().phase)
        assertEquals(1, repo.state.value.busy.size)

        node.listener.transferFinished("t1", TransferResult.COMPLETED, null, listOf("/data/incoming/a.jpg"))
        assertEquals(BeamRepository.SAVING, repo.state.value.transfers.single().status)
        advanceUntilIdle()
        val t = repo.state.value.transfers.single()
        assertEquals(Phase.DONE, t.phase)
        assertEquals(listOf("content://media/downloads/a.jpg"), t.savedUris)
        assertNull(t.status)
        assertEquals(listOf("/data/incoming/a.jpg"), saved.single())
        assertTrue(repo.state.value.busy.isEmpty())
    }

    @Test
    fun anInterruptedReceiveDoesNotHoldTheLocks() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        node.listener.transferStarted(TransferInfo("t1", Direction.RECEIVE, "ThinkPad", listOf("a.jpg"), 1u, 1000u))
        node.listener.transferFinished("t1", TransferResult.INTERRUPTED, "connection lost", emptyList())
        assertEquals(Phase.INTERRUPTED, repo.state.value.transfers.single().phase)
        assertTrue(repo.state.value.busy.isEmpty())
    }

    @Test
    fun aFailedSendShowsWhyAndCleansUp() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        node.sendResult = { throw BeamException.Failed("\"ThinkPad\" didn't answer") }
        var cleaned = false
        val file = OutgoingFile("a.jpg", 0, FileSource.Path("/cache/a.jpg"))
        val id = repo.send("aaaa-bbbb-cccc-dddd", "ThinkPad", listOf(file)) { cleaned = true }
        advanceUntilIdle()
        val t = repo.state.value.transfers.single()
        assertEquals(id, t.id)
        assertEquals(Phase.FAILED, t.phase)
        assertEquals("\"ThinkPad\" didn't answer", t.status)
        assertTrue(cleaned)
    }

    @Test
    fun nearbyDevicesAreSortedAndRemoved() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        node.listener.deviceFound(NearbyDevice("2", "zenbook", DeviceKind.LAPTOP, null, false))
        node.listener.deviceFound(NearbyDevice("1", "ThinkPad", DeviceKind.LAPTOP, null, true))
        assertEquals(listOf("ThinkPad", "zenbook"), repo.state.value.nearby.map { it.name })
        node.listener.deviceLost("2")
        assertEquals(listOf("ThinkPad"), repo.state.value.nearby.map { it.name })
    }

    @Test
    fun pairingTrustsTheLaptopAndErrorsAreReadable() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        assertEquals("ThinkPad", repo.pair("beam://pair?v=1").getOrThrow().name)
        assertEquals(listOf("ThinkPad"), repo.state.value.trusted.map { it.name })

        node.pairResult = { throw BeamException.Failed("pairing refused: this code was already used") }
        assertEquals("pairing refused: this code was already used", repo.pair("beam://pair?v=1").exceptionOrNull()?.message)
    }

    @Test
    fun stoppingClearsWhatIsNoLongerTrue() = runTest(dispatcher) {
        val repo = repository()
        repo.start()
        node.listener.deviceFound(NearbyDevice("1", "ThinkPad", DeviceKind.LAPTOP, null, true))
        repo.stop()
        assertFalse(repo.state.value.running)
        assertTrue(repo.state.value.nearby.isEmpty())
        assertTrue("stop" in node.calls)
        assertFalse(discovery.started)
    }

    @Test
    fun aNodeThatCannotStartReportsTheError() = runTest(dispatcher) {
        val repo = BeamRepository(
            // Not backgroundScope: advanceUntilIdle doesn't run work launched there.
            scope = this,
            settings = settings,
            createNode = { _, _ -> throw BeamException.Failed("could not start receiving: address in use") },
            platformDiscovery = null,
            saveReceived = { emptyList() },
            io = dispatcher,
        )
        repo.start()
        assertFalse(repo.state.value.running)
        assertEquals("could not start receiving: address in use", repo.state.value.error)
    }

    class FakeDiscovery : PlatformDiscovery {
        var sink: PlatformDiscovery.Sink? = null
        val started get() = sink != null
        val adverts = mutableListOf<ServiceAdvert?>()

        override fun start(sink: PlatformDiscovery.Sink) {
            this.sink = sink
        }

        override fun advertise(advert: ServiceAdvert?) {
            adverts += advert
        }

        override fun stop() {
            sink = null
        }
    }
}
