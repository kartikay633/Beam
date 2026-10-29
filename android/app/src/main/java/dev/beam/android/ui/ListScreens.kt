package dev.beam.android.ui

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material3.AlertDialog
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.beam.android.data.BeamState
import dev.beam.android.data.DiscoveryChoice
import dev.beam.core.TrustedDevice
import kotlin.math.roundToInt

@Composable
fun TransfersScreen(state: BeamState, vm: BeamViewModel) {
    LazyColumn(Modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        item { Spacer(Modifier.height(8.dp)) }
        if (state.transfers.isEmpty()) {
            item {
                Text(
                    "Files you send and receive show up here.",
                    fontSize = 14.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(4.dp),
                )
            }
        } else {
            item {
                ListCard {
                    state.transfers.forEachIndexed { i, t ->
                        if (i > 0) RowDivider()
                        TransferRow(t) { vm.cancel(t.id) }
                    }
                }
            }
        }
        item { Spacer(Modifier.height(24.dp)) }
    }
}

@Composable
fun SettingsScreen(state: BeamState, vm: BeamViewModel, onPair: () -> Unit) {
    val context = LocalContext.current
    var name by remember { mutableStateOf(vm.deviceName) }
    var streams by remember { mutableFloatStateOf(vm.streams.toFloat()) }
    var discovery by remember { mutableStateOf(vm.discovery) }
    var removing by remember { mutableStateOf<TrustedDevice?>(null) }

    Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
        SectionLabel("This phone")
        ListCard {
            Column(Modifier.padding(16.dp)) {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it.take(63) },
                    label = { Text("Name others see") },
                    singleLine = true,
                    shape = RoundedCornerShape(10.dp),
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 4.dp)) {
                    if (state.deviceId.isNotEmpty()) {
                        Text(
                            state.deviceId,
                            fontSize = 12.sp,
                            fontFamily = FontFamily.Monospace,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.weight(1f),
                        )
                    } else {
                        Spacer(Modifier.weight(1f))
                    }
                    TextButton(onClick = { vm.setDeviceName(name) }, enabled = name.isNotBlank() && name.trim() != vm.deviceName) {
                        Text("Save name")
                    }
                }
            }
        }

        SectionLabel("Trusted devices", action = "Pair", onAction = onPair)
        Text(
            "Trusted devices can always find this phone. With auto-accept on, their files arrive without asking.",
            fontSize = 13.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 4.dp, end = 4.dp, bottom = 10.dp),
        )
        ListCard {
            if (state.trusted.isEmpty()) {
                Row(
                    Modifier.fillMaxWidth().clickable(onClick = onPair).padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    IconTile(Icons.Filled.QrCodeScanner, size = 40.dp)
                    Text("Pair with a laptop", fontSize = 15.sp, fontWeight = FontWeight.Medium, modifier = Modifier.weight(1f))
                    Chevron()
                }
            }
            state.trusted.forEachIndexed { i, device ->
                if (i > 0) RowDivider()
                Column(Modifier.padding(start = 16.dp, end = 8.dp, top = 12.dp, bottom = 8.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(device.name, fontSize = 15.sp, fontWeight = FontWeight.Medium)
                            Text(
                                device.deviceId,
                                fontSize = 12.sp,
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        TextButton(onClick = { removing = device }) { Text("Remove", color = MaterialTheme.colorScheme.error) }
                    }
                    Row(
                        Modifier.fillMaxWidth().heightIn(min = 48.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text("Accept files automatically", fontSize = 14.sp, modifier = Modifier.weight(1f))
                        Switch(checked = device.autoAccept, onCheckedChange = { vm.setAutoAccept(device.deviceId, it) })
                        Spacer(Modifier.padding(end = 8.dp))
                    }
                }
            }
        }

        SectionLabel("Speed")
        ListCard {
            Column(Modifier.padding(16.dp)) {
                Text("Parallel connections when sending: ${streams.roundToInt()}", fontSize = 14.sp)
                Slider(
                    value = streams,
                    onValueChange = { streams = it },
                    onValueChangeFinished = { vm.setStreams(streams.roundToInt()) },
                    valueRange = 1f..8f,
                    steps = 6,
                )
                Text(
                    "More connections help on fast Wi-Fi. 4 is a good default.",
                    fontSize = 13.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        SectionLabel("Finding devices on Wi-Fi")
        ListCard {
            DiscoveryChoice.entries.forEachIndexed { i, choice ->
                if (i > 0) RowDivider()
                Row(
                    Modifier.fillMaxWidth().selectable(
                        selected = discovery == choice,
                        role = Role.RadioButton,
                        onClick = {
                            discovery = choice
                            vm.setDiscovery(choice)
                        },
                    ).padding(horizontal = 8.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = discovery == choice, onClick = null, modifier = Modifier.padding(8.dp))
                    Column(Modifier.padding(start = 4.dp, end = 8.dp)) {
                        Text(
                            when (choice) {
                                DiscoveryChoice.ANDROID_NSD -> "Android's discovery (recommended)"
                                DiscoveryChoice.BUILT_IN -> "Beam's own"
                            },
                            fontSize = 14.sp,
                            fontWeight = FontWeight.Medium,
                        )
                        Text(
                            when (choice) {
                                DiscoveryChoice.ANDROID_NSD -> "Uses the system's discovery service."
                                DiscoveryChoice.BUILT_IN -> "The same code as the desktop app. Try it if devices don't show up."
                            },
                            fontSize = 13.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        Text(
            "Beam ${versionName(context)} · Received files go to Download/Beam",
            fontSize = 12.sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 4.dp, top = 20.dp, bottom = 28.dp),
        )
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

private fun versionName(context: Context): String =
    runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: ""
