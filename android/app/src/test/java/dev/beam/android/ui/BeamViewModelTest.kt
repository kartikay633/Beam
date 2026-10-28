package dev.beam.android.ui

import dev.beam.android.data.BeamRepository
import dev.beam.android.data.FakeNode
import dev.beam.android.data.FileAccess
import dev.beam.android.data.MemorySettings
import dev.beam.android.data.OpenedFiles
import dev.beam.android.data.StagedFile
import dev.beam.core.DeviceKind
import dev.beam.core.FileSource
import dev.beam.core.NearbyDevice
import dev.beam.core.NearbyLink
import dev.beam.core.OutgoingFile
import java.io.IOException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class BeamViewModelTest {
    private val dispatcher = StandardTestDispatcher()
    private val files = FakeFiles()
    private val settings = MemorySettings()
    private lateinit var node: FakeNode
    private val laptop = NearbyDevice("aaaa-bbbb-cccc-dddd", "ThinkPad", DeviceKind.LAPTOP, "192.168.1.20:40000", true, NearbyLink.LAN, null, null)

    @Before
    fun setUp() = Dispatchers.setMain(dispatcher)

    @After
    fun tearDown() = Dispatchers.resetMain()

    private fun TestScope.viewModel(): Pair<BeamViewModel, BeamRepository> {
        val repo = BeamRepository(
            // Not backgroundScope: advanceUntilIdle doesn't run work launched there.
            scope = this,
            settings = settings,
            createNode = { s, listener -> FakeNode(listener, s.visibility).also { node = it } },
            platformDiscovery = null,
            saveReceived = { emptyList() },
            io = dispatcher,
        )
        return BeamViewModel(repo, files, settings, io = dispatcher) to repo
    }

    @Test
    fun sharedFilesAreStagedOnceEach() = runTest(dispatcher) {
        val (vm, _) = viewModel()
        vm.stage(listOf("content://a", "content://b"))
        vm.stage(listOf("content://a"))
        advanceUntilIdle()
        assertEquals(listOf("a", "b"), vm.staged.value.map { it.name })
        vm.unstage("content://a")
        assertEquals(listOf("b"), vm.staged.value.map { it.name })
    }

    @Test
    fun unreadableFilesAreSkippedWithAMessage() = runTest(dispatcher) {
        val (vm, repo) = viewModel()
        vm.stage(listOf("content://a", "content://broken"))
        advanceUntilIdle()
        assertEquals(listOf("a"), vm.staged.value.map { it.name })
        assertEquals("Some files couldn't be read", repo.state.value.message)
    }

    @Test
    fun sendingOpensTheStagedFilesAndClearsThem() = runTest(dispatcher) {
        val (vm, repo) = viewModel()
        repo.start()
        vm.stage(listOf("content://a", "content://b"))
        advanceUntilIdle()
        vm.sendTo(laptop)
        advanceUntilIdle()
        assertTrue(vm.staged.value.isEmpty())
        val (target, sent) = node.sent.single()
        assertEquals(laptop.deviceId, target)
        assertEquals(listOf("a", "b"), sent.map { it.name })
        assertTrue(files.cleanedUp)
    }

    @Test
    fun pickingFromADeviceRowSendsStraightAway() = runTest(dispatcher) {
        val (vm, repo) = viewModel()
        repo.start()
        vm.stageAndSend(listOf("content://a"), laptop)
        advanceUntilIdle()
        assertEquals(listOf("a"), node.sent.single().second.map { it.name })
    }

    @Test
    fun filesThatCannotBeOpenedStayStaged() = runTest(dispatcher) {
        val (vm, repo) = viewModel()
        repo.start()
        files.failOpen = true
        vm.stage(listOf("content://a"))
        advanceUntilIdle()
        vm.sendTo(laptop)
        advanceUntilIdle()
        assertEquals(listOf("a"), vm.staged.value.map { it.name })
        assertTrue(node.sent.isEmpty())
        assertTrue(repo.state.value.message!!.startsWith("Couldn't read the files"))
    }

    @Test
    fun renamingRestartsTheNode() = runTest(dispatcher) {
        val (vm, repo) = viewModel()
        repo.start()
        val first = node
        vm.setDeviceName("Kartikay's Pixel")
        advanceUntilIdle()
        assertTrue("stop" in first.calls)
        assertTrue(node !== first)
        assertEquals("Kartikay's Pixel", settings.deviceName)
    }

    class FakeFiles : FileAccess {
        var failOpen = false
        var cleanedUp = false

        override fun describe(uri: String): StagedFile? {
            if (uri.endsWith("broken")) throw SecurityException("no permission")
            return StagedFile(uri, uri.substringAfterLast('/'), 10)
        }

        override fun open(files: List<StagedFile>): OpenedFiles {
            if (failOpen) throw IOException("provider crashed")
            return OpenedFiles(files.map { OutgoingFile(it.name, 0, FileSource.Path("/cache/${it.name}")) }) { cleanedUp = true }
        }
    }
}
