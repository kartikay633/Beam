package dev.beam.android.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.beam.android.data.BeamState
import dev.beam.android.data.StagedFile
import dev.beam.android.model.Format
import dev.beam.android.service.Notifications
import dev.beam.core.NearbyDevice
import dev.beam.core.Visibility
import kotlinx.coroutines.delay

@Composable
fun HomeScreen(
    state: BeamState,
    staged: List<StagedFile>,
    vm: BeamViewModel,
    notificationBanner: @Composable () -> Unit,
    onRetry: () -> Unit,
    onPair: () -> Unit,
    onSettings: () -> Unit,
    onShowTransfers: () -> Unit,
) {
    // Files picked from a device row go straight to that device.
    var pickFor by remember { mutableStateOf<NearbyDevice?>(null) }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
        val target = pickFor
        pickFor = null
        val strings = uris.map { it.toString() }
        if (target != null) vm.stageAndSend(strings, target) else vm.stage(strings)
    }
    val pick = { picker.launch(arrayOf("*/*")) }

    Column(Modifier.fillMaxSize()) {
        LazyColumn(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 16.dp)) {
            item {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 12.dp, start = 4.dp)) {
                    BeamLogo(28.dp)
                    Text(
                        "Beam",
                        fontSize = 22.sp,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.padding(start = 10.dp).weight(1f),
                    )
                    IconButton(onClick = onPair) { Icon(Icons.Filled.QrCodeScanner, "Pair with a laptop") }
                    IconButton(onClick = onSettings) { Icon(Icons.Filled.Tune, "Settings") }
                }
            }
            item {
                Row(Modifier.padding(top = 10.dp, start = 4.dp)) {
                    VisibilityChip(state.visibility, visibilityLabel(state), vm::setVisibility)
                }
            }
            item { notificationBanner() }
            state.error?.let { error ->
                item { Banner("Beam couldn't start: $error", action = "Retry", error = true, onAction = onRetry) }
            }

            if (staged.isNotEmpty()) {
                item { StagedFiles(staged, onAdd = pick, onRemove = vm::unstage, onClear = vm::clearStaged) }
            }

            item { SectionLabel(if (staged.isEmpty()) "Send to" else "Choose a device") }
            item {
                ListCard {
                    if (state.nearby.isEmpty()) {
                        Row(
                            Modifier.padding(16.dp),
                            horizontalArrangement = Arrangement.spacedBy(14.dp),
                        ) {
                            Column(Modifier.padding(top = 6.dp)) { SearchingDot() }
                            Column(Modifier.weight(1f)) {
                                Text(
                                    if (state.running) "Looking for devices…" else "Beam isn't running",
                                    fontSize = 15.sp,
                                    fontWeight = FontWeight.Medium,
                                )
                                Text(
                                    "Open Beam on your laptop. It shows up here on the same Wi-Fi, or over Bluetooth with no shared network.",
                                    fontSize = 13.sp,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.padding(top = 2.dp),
                                )
                                TextButton(onClick = onPair, modifier = Modifier.padding(top = 4.dp)) {
                                    Text("Pair with a laptop")
                                }
                            }
                        }
                    } else {
                        state.nearby.forEachIndexed { i, device ->
                            if (i > 0) RowDivider()
                            val send = {
                                if (staged.isEmpty()) {
                                    pickFor = device
                                    pick()
                                } else {
                                    vm.sendTo(device)
                                }
                            }
                            DeviceRow(device, onClick = send) {
                                if (staged.isEmpty()) Chevron() else RowButton("Send", send)
                            }
                        }
                    }
                }
            }

            val recent = state.transfers.take(3)
            if (recent.isNotEmpty()) {
                item { SectionLabel("Recent", action = "See all", onAction = onShowTransfers) }
                item {
                    ListCard {
                        recent.forEachIndexed { i, t ->
                            if (i > 0) RowDivider()
                            TransferRow(t) { vm.cancel(t.id) }
                        }
                    }
                }
            }
            item { Spacer(Modifier.height(16.dp)) }
        }

        if (staged.isEmpty()) {
            PrimaryButton(
                "Choose files to send",
                onClick = pick,
                icon = Icons.Filled.Add,
                modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 16.dp),
            )
        }
    }
}

/** Files shared to Beam or picked, waiting for a device. */
@Composable
private fun StagedFiles(staged: List<StagedFile>, onAdd: () -> Unit, onRemove: (String) -> Unit, onClear: () -> Unit) {
    Column {
        Row(
            Modifier.fillMaxWidth().padding(start = 4.dp, top = 18.dp, bottom = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text("Send ${Notifications.files(staged.size)}", fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                val total = staged.mapNotNull { it.size }.sum()
                if (total > 0) Text(Format.bytes(total), fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            TextButton(onClick = onAdd) { Text("Add more") }
            TextButton(onClick = onClear) { Text("Clear") }
        }
        ListCard {
            staged.take(5).forEachIndexed { i, file ->
                if (i > 0) RowDivider()
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 52.dp).padding(start = 16.dp, end = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(file.name, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    file.size?.let {
                        Text(Format.bytes(it), fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.width(4.dp))
                    }
                    IconButton(onClick = { onRemove(file.uri) }) {
                        Icon(Icons.Filled.Close, "Remove ${file.name}", tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            if (staged.size > 5) {
                RowDivider()
                Text(
                    "and ${staged.size - 5} more",
                    fontSize = 13.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                )
            }
        }
    }
}

/** "Visible to everyone · 9 min", ticking down. */
@Composable
private fun visibilityLabel(state: BeamState): String {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(state.everyoneUntilMs) {
        while (state.everyoneUntilMs != null) {
            now = System.currentTimeMillis()
            delay(15_000)
        }
    }
    return when (state.visibility) {
        Visibility.EVERYONE -> {
            val left = state.everyoneUntilMs?.let { ((it - now) / 60_000.0).let { m -> kotlin.math.ceil(m).toInt().coerceAtLeast(1) } }
            if (left != null) "Visible to everyone · $left min" else "Visible to everyone"
        }
        Visibility.TRUSTED_ONLY -> "Visible to trusted devices"
        Visibility.OFF -> "Hidden"
    }
}
