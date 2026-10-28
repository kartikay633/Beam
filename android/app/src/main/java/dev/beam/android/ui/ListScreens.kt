package dev.beam.android.ui

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.beam.android.data.BeamState
import dev.beam.android.data.DiscoveryChoice
import dev.beam.core.TrustedDevice
import kotlin.math.roundToInt

@Composable
fun TransfersScreen(state: BeamState, vm: BeamViewModel) {
    LazyColumn(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                SectionTitle("Transfers", Modifier.weight(1f))
                if (state.transfers.any { it.finished }) TextButton(onClick = vm::clearFinished) { Text("Clear finished") }
            }
        }
        if (state.transfers.isEmpty()) {
            item { Text("Nothing sent or received yet.", color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        items(state.transfers, key = { it.id }) { t -> TransferCard(t) { vm.cancel(t.id) } }
    }
}

@Composable
fun TrustedScreen(state: BeamState, vm: BeamViewModel, onPair: () -> Unit) {
    var removing by remember { mutableStateOf<TrustedDevice?>(null) }
    LazyColumn(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        item {
            SectionTitle("Trusted devices")
            Text(
                "Trusted devices can see this phone when it's set to Trusted, and with auto-accept their files arrive without asking.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Button(onClick = onPair, modifier = Modifier.padding(vertical = 12.dp)) {
                Icon(Icons.Filled.QrCodeScanner, null)
                Text("  Pair with a laptop")
            }
        }
        if (state.trusted.isEmpty()) {
            item { Text("No trusted devices yet.", color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        items(state.trusted, key = { it.deviceId }) { device ->
            ListItem(
                headlineContent = { Text(device.name) },
                supportingContent = {
                    Column {
                        Text(device.deviceId, fontFamily = FontFamily.Monospace, style = MaterialTheme.typography.bodySmall)
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text("Auto-accept", Modifier.weight(1f))
                            Switch(checked = device.autoAccept, onCheckedChange = { vm.setAutoAccept(device.deviceId, it) })
                        }
                    }
                },
                trailingContent = { TextButton(onClick = { removing = device }) { Text("Remove") } },
            )
            HorizontalDivider()
        }
    }
    removing?.let { device ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("Remove ${device.name}?") },
            text = { Text("It will have to ask before sending, and you'll need to pair again to trust it.") },
            confirmButton = {
                TextButton(onClick = {
                    vm.removeTrusted(device.deviceId)
                    removing = null
                }) { Text("Remove") }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("Keep") } },
        )
    }
}

@Composable
fun SettingsScreen(state: BeamState, vm: BeamViewModel) {
    val context = LocalContext.current
    var name by remember { mutableStateOf(vm.deviceName) }
    var streams by remember { mutableFloatStateOf(vm.streams.toFloat()) }
    var discovery by remember { mutableStateOf(vm.discovery) }

    Column(
        Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        SectionTitle("This phone")
        OutlinedTextField(
            value = name,
            onValueChange = { name = it.take(63) },
            label = { Text("Name others see") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        TextButton(onClick = { vm.setDeviceName(name) }, enabled = name.isNotBlank() && name.trim() != vm.deviceName) {
            Text("Save name")
        }
        if (state.deviceId.isNotEmpty()) {
            Text("Device id ${state.deviceId}", style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
        }

        SectionTitle("Speed")
        Text("Parallel connections when sending: ${streams.roundToInt()}")
        Slider(
            value = streams,
            onValueChange = { streams = it },
            onValueChangeFinished = { vm.setStreams(streams.roundToInt()) },
            valueRange = 1f..8f,
            steps = 6,
        )
        Text(
            "More connections help on fast Wi-Fi; 4 is a good default.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        SectionTitle("Finding devices")
        DiscoveryChoice.entries.forEach { choice ->
            Row(
                Modifier.fillMaxWidth().selectable(
                    selected = discovery == choice,
                    role = Role.RadioButton,
                    onClick = {
                        discovery = choice
                        vm.setDiscovery(choice)
                    },
                ),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(selected = discovery == choice, onClick = null)
                Spacer(Modifier.padding(4.dp))
                Column {
                    Text(
                        when (choice) {
                            DiscoveryChoice.ANDROID_NSD -> "Android (NsdManager)"
                            DiscoveryChoice.BUILT_IN -> "Beam's own mDNS"
                        },
                    )
                    Text(
                        when (choice) {
                            DiscoveryChoice.ANDROID_NSD -> "Uses the system's discovery service. Recommended."
                            DiscoveryChoice.BUILT_IN -> "The same code as the desktop app. Try it if devices don't show up."
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }

        SectionTitle("About")
        Text("Beam ${versionName(context)}", style = MaterialTheme.typography.bodySmall)
        Text(
            "Received files are saved to Download/Beam.",
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.padding(bottom = 24.dp),
        )
    }
}

private fun versionName(context: Context): String =
    runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: ""
