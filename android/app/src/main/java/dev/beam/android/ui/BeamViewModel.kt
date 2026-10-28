package dev.beam.android.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import dev.beam.android.BeamApp
import dev.beam.android.data.BeamRepository
import dev.beam.android.data.BeamState
import dev.beam.android.data.DiscoveryChoice
import dev.beam.android.data.FileAccess
import dev.beam.android.data.SettingsStore
import dev.beam.android.data.StagedFile
import dev.beam.core.NearbyDevice
import dev.beam.core.Visibility
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Screen state on top of [BeamRepository]: the files waiting to be sent, and settings edits. */
class BeamViewModel(
    private val repository: BeamRepository,
    private val files: FileAccess,
    private val settings: SettingsStore,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {
    val state: StateFlow<BeamState> = repository.state

    private val _staged = MutableStateFlow<List<StagedFile>>(emptyList())
    val staged: StateFlow<List<StagedFile>> = _staged.asStateFlow()

    /** Adds shared or picked files (content URIs). */
    fun stage(uris: List<String>) {
        if (uris.isEmpty()) return
        viewModelScope.launch {
            val described = withContext(io) { uris.mapNotNull { runCatching { files.describe(it) }.getOrNull() } }
            _staged.update { (it + described).distinctBy { file -> file.uri } }
            if (described.size < uris.size) repository.showMessage("Some files couldn't be read")
        }
    }

    /** Picked straight from a device row: stage, then send to it. */
    fun stageAndSend(uris: List<String>, device: NearbyDevice) {
        if (uris.isEmpty()) return
        viewModelScope.launch {
            val described = withContext(io) { uris.mapNotNull { runCatching { files.describe(it) }.getOrNull() } }
            _staged.update { (it + described).distinctBy { file -> file.uri } }
            sendTo(device)
        }
    }

    fun unstage(uri: String) =_staged.update { list -> list.filterNot { it.uri == uri } }

    fun clearStaged() = _staged.update { emptyList() }

    /** Sends the staged files to [device]. */
    fun sendTo(device: NearbyDevice) {
        val toSend = _staged.value
        if (toSend.isEmpty()) return
        _staged.value = emptyList()
        viewModelScope.launch {
            val opened = withContext(io) { runCatching { files.open(toSend) } }
            opened
                .onSuccess { repository.send(device, it.files, it.cleanup) }
                .onFailure {
                    _staged.value = toSend
                    repository.showMessage("Couldn't read the files: ${BeamRepository.describe(it)}")
                }
        }
    }

    fun setVisibility(visibility: Visibility) = repository.setVisibility(visibility)
    fun refreshRadio() = repository.refreshRadio()
    fun cancel(transferId: String) = repository.cancel(transferId)
    fun respond(offerId: String, accept: Boolean, trust: Boolean) = repository.respond(offerId, accept, trust)
    fun clearFinished() = repository.clearFinished()
    fun messageShown() = repository.messageShown()
    fun setAutoAccept(deviceId: String, on: Boolean) = repository.setAutoAccept(deviceId, on)
    fun removeTrusted(deviceId: String) = repository.removeTrusted(deviceId)
    suspend fun pair(code: String) = repository.pair(code)

    val deviceName get() = settings.deviceName
    val streams get() = settings.streams
    val discovery get() = settings.discovery

    fun setStreams(streams: Int) = repository.setStreams(streams)

    fun setDeviceName(name: String) {
        if (name.trim() == settings.deviceName) return
        settings.deviceName = name
        viewModelScope.launch { repository.restart() }
    }

    fun setDiscovery(choice: DiscoveryChoice) {
        if (choice == settings.discovery) return
        settings.discovery = choice
        viewModelScope.launch { repository.restart() }
    }

    companion object {
        val Factory: ViewModelProvider.Factory = viewModelFactory {
            initializer {
                val app = this[ViewModelProvider.AndroidViewModelFactory.APPLICATION_KEY] as BeamApp
                BeamViewModel(app.repository, app.files, app.settings)
            }
        }
    }
}
