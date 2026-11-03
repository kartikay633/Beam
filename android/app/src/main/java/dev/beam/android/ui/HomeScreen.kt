package dev.beam.android.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material3.Icon
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.AlertDialog
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
import dev.beam.android.data.saveKey
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
                    if (state.trusted.isEmpty()) {
                        TextButton(onClick = onPair) {
                            Icon(Icons.Filled.QrCodeScanner, null, modifier = Modifier.padding(end = 6.dp))
                            Text("Scan code")
                        }
                    } else {
                        IconButton(onClick = onPair) { Icon(Icons.Filled.QrCodeScanner, "Pair with a computer") }
                    }
                    IconButton(onClick = onSettings) { Icon(Icons.Filled.Tune, "Settings") }
                }
            }
            item {
                Row(Modifier.padding(top = 10.dp, start = 4.dp)) {
                    VisibilityChip(state.visibility, visibilityLabel(state), vm::setVisibility)
                }
            }
            item { notificationBanner() }
            if (state.trusted.isEmpty()) {
                item { PairHint(onPair) }
            }
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
                                RowButton(if (staged.isEmpty()) "Send files" else "Send", send)
                            }
                            val key = saveKey(device.name)
                            val folders = state.saveFolders[key].orEmpty()
                            if (device.trusted && folders.isNotEmpty()) {
                                SaveToChoice(folders, state.saveTo[key]) { vm.setSaveTo(device, it) }
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

/** "Save to  Desktop ▾": which folder on the PC the files land in. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SaveToChoice(folders: List<String>, chosen: String?, onChoose: (String?) -> Unit) {
    var open by remember { mutableStateOf(false) }
    var newIn by remember { mutableStateOf<String?>(null) }
    Row(
        Modifier.fillMaxWidth().padding(start = 70.dp, end = 16.dp, bottom = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text("Save to", fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
        TextButton(onClick = { open = true }) {
            Text(folderLabel(chosen), fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Icon(Icons.Filled.ArrowDropDown, null)
        }
    }
    if (open) {
        val pick = { folder: String? ->
            open = false
            onChoose(folder)
        }
        ModalBottomSheet(onDismissRequest = { open = false }) {
            LazyColumn(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
                item { SheetRow(folderLabel(null), chosen == null, indent = false) { pick(null) } }
                saveFolderTree(folders).forEach { (root, subs) ->
                    item { SheetRow(folderLabel(root), chosen == root, indent = false) { pick(root) } }
                    items(subs) { sub -> SheetRow(sub.substringAfter('/'), chosen == sub, indent = true) { pick(sub) } }
                    item {
                        SheetRow("New folder…", selected = false, indent = true, accent = true) {
                            open = false
                            newIn = root
                        }
                    }
                }
            }
        }
    }
    newIn?.let { root ->
        NewFolderDialog(
            inFolder = folderLabel(root),
            onDismiss = { newIn = null },
            onCreate = { name ->
                newIn = null
                onChoose("$root/$name")
            },
        )
    }
}

@Composable
private fun SheetRow(text: String, selected: Boolean, indent: Boolean, accent: Boolean = false, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).heightIn(min = 48.dp)
            .padding(start = if (indent) 44.dp else 20.dp, end = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text,
            fontSize = 15.sp,
            fontWeight = if (indent) FontWeight.Normal else FontWeight.Medium,
            color = if (accent) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        if (selected) Icon(Icons.Filled.Check, "Selected", tint = MaterialTheme.colorScheme.primary)
    }
}

@Composable
private fun NewFolderDialog(inFolder: String, onDismiss: () -> Unit, onCreate: (String) -> Unit) {
    var name by remember { mutableStateOf("") }
    val problem = folderNameProblem(name)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("New folder in $inFolder") },
        text = {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                singleLine = true,
                label = { Text("Folder name") },
                isError = name.isNotEmpty() && problem != null,
                supportingText = { if (name.isNotEmpty() && problem != null) Text(problem) },
            )
        },
        confirmButton = { TextButton(onClick = { onCreate(name.trim()) }, enabled = problem == null) { Text("Use folder") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** A PC's folders with their listed subfolders: ("documents", ["documents/Projects", …]). */
internal fun saveFolderTree(folders: List<String>): List<Pair<String, List<String>>> {
    val roots = folders.filter { '/' !in it }
    return roots.map { root -> root to folders.filter { it.startsWith("$root/") } }
}

/** Characters Windows doesn't allow in a folder name. */
private const val FORBIDDEN = "<>:\"/\\|?*"

/** Why [raw] can't be a new folder on the PC, or null if it can. The PC checks again. */
internal fun folderNameProblem(raw: String): String? {
    val name = raw.trim()
    return when {
        name.isEmpty() -> "Type a name"
        name.length > 100 -> "Use a shorter name"
        name.startsWith('.') || name.endsWith('.') -> "Can't start or end with a dot"
        name.any { it in FORBIDDEN || it.code < 32 } -> "Can't contain any of $FORBIDDEN"
        else -> null
    }
}

/** A PC folder id as people say it; null is the PC's own Beam folder. */
internal fun folderLabel(id: String?): String = when {
    id == null -> "Beam folder"
    // A subfolder: "documents/Projects" → "Documents › Projects".
    '/' in id -> "${folderLabel(id.substringBefore('/'))} › ${id.substringAfter('/')}"
    // A folder the PC's owner pinned: "pin:Projects".
    id.startsWith("pin:") -> id.removePrefix("pin:").ifBlank { "Pinned folder" }
    id == "downloads" -> "Downloads"
    id == "desktop" -> "Desktop"
    id == "documents" -> "Documents"
    id == "pictures" -> "Pictures"
    id == "videos" -> "Videos"
    else -> id.replaceFirstChar { it.uppercase() }
}

/** Shown until a computer is paired: the three steps, and the button to start. */
@Composable
private fun PairHint(onPair: () -> Unit) {
    ListCard(Modifier.padding(top = 14.dp)) {
        Row(
            Modifier.fillMaxWidth().padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            IconTile(Icons.Filled.QrCodeScanner)
            Column(Modifier.weight(1f)) {
                Text("Pair your computer", fontSize = 16.sp, fontWeight = FontWeight.SemiBold)
                Text(
                    "1. On your computer, open Beam and click Your devices. A square code appears.\n" +
                        "2. Tap Scan code and point your camera at it.",
                    fontSize = 14.sp,
                    lineHeight = 20.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(top = 4.dp),
                )
                RowButton("Scan code", onPair)
            }
        }
    }
}
