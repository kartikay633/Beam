package dev.beam.android.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.Laptop
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Verified
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.beam.android.data.BeamState
import dev.beam.android.data.StagedFile
import dev.beam.android.model.Format
import dev.beam.android.service.Notifications
import dev.beam.core.DeviceKind
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

    LazyColumn(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        item { VisibilityCard(state, vm::setVisibility) }
        item { notificationBanner() }
        state.error?.let { error ->
            item { Banner("Beam couldn't start: $error", action = "Retry", error = true, onAction = onRetry) }
        }

        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SectionTitle(if (staged.isEmpty()) "Send" else "Ready to send", Modifier.weight(1f))
                if (staged.isNotEmpty()) TextButton(onClick = vm::clearStaged) { Text("Clear") }
            }
        }
        if (staged.isEmpty()) {
            item {
                OutlinedButton(onClick = pick, modifier = Modifier.fillMaxWidth()) { Text("Choose files") }
                Text(
                    "Or share files to Beam from any app, then pick a device.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(top = 6.dp),
                )
            }
        } else {
            items(staged, key = { it.uri }) { file ->
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text(file.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        file.size?.let { Text(Format.bytes(it), style = MaterialTheme.typography.bodySmall) }
                    }
                    IconButton(onClick = { vm.unstage(file.uri) }) { Icon(Icons.Filled.Close, "Remove ${file.name}") }
                }
            }
            item {
                val total = staged.mapNotNull { it.size }.sum()
                Text(
                    "${Notifications.files(staged.size)} · ${Format.bytes(total)}. Tap a device below to send.",
                    style = MaterialTheme.typography.bodySmall,
                )
                TextButton(onClick = pick) { Text("Add more") }
            }
        }

        item { SectionTitle("Nearby") }
        if (state.nearby.isEmpty()) {
            item {
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(
                            if (state.running) "Looking for devices on this Wi-Fi…" else "Beam isn't running.",
                            style = MaterialTheme.typography.bodyLarge,
                        )
                        Text(
                            "Open Beam on your laptop. If it's set to trusted devices only, pair with it first.",
                            style = MaterialTheme.typography.bodySmall,
                        )
                        Button(onClick = onPair) {
                            Icon(Icons.Filled.QrCodeScanner, null)
                            Text("  Pair with a laptop")
                        }
                    }
                }
            }
        } else {
            items(state.nearby, key = { it.deviceId }) { device ->
                ListItem(
                    headlineContent = { Text(device.name) },
                    supportingContent = {
                        Text(if (staged.isEmpty()) "Tap to choose files to send" else "Tap to send ${Notifications.files(staged.size)}")
                    },
                    leadingContent = {
                        Icon(
                            when (device.kind) {
                                DeviceKind.LAPTOP -> Icons.Filled.Laptop
                                DeviceKind.PHONE -> Icons.Filled.PhoneAndroid
                                DeviceKind.UNKNOWN -> Icons.Filled.Devices
                            },
                            null,
                        )
                    },
                    trailingContent = { if (device.trusted) Icon(Icons.Filled.Verified, "Trusted") },
                    modifier = Modifier.clickable {
                        if (staged.isEmpty()) {
                            pickFor = device
                            pick()
                        } else {
                            vm.sendTo(device)
                        }
                    },
                )
            }
        }

        val recent = state.transfers.take(3)
        if (recent.isNotEmpty()) {
            item {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    SectionTitle("Recent transfers", Modifier.weight(1f))
                    TextButton(onClick = onShowTransfers) { Text("See all") }
                }
            }
            items(recent, key = { it.id }) { t -> TransferCard(t) { vm.cancel(t.id) } }
        }
        item { androidx.compose.foundation.layout.Spacer(Modifier.padding(16.dp)) }
    }
}

@Composable
private fun VisibilityCard(state: BeamState, onChange: (Visibility) -> Unit) {
    // Ticks the "for N min" countdown.
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(state.everyoneUntilMs) {
        while (state.everyoneUntilMs != null) {
            now = System.currentTimeMillis()
            delay(15_000)
        }
    }
    Column(Modifier.padding(top = 16.dp)) {
        Text("Who can see this phone", style = MaterialTheme.typography.titleMedium)
        val options = listOf(Visibility.EVERYONE to "Everyone", Visibility.TRUSTED_ONLY to "Trusted", Visibility.OFF to "Off")
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(top = 8.dp)) {
            options.forEachIndexed { index, (value, label) ->
                SegmentedButton(
                    selected = state.visibility == value,
                    onClick = { onChange(value) },
                    shape = SegmentedButtonDefaults.itemShape(index, options.size),
                ) { Text(label) }
            }
        }
        Text(
            Notifications.visibilityText(state, now),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 6.dp),
        )
    }
}
