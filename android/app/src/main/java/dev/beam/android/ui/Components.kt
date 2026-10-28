package dev.beam.android.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import androidx.core.net.toUri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.beam.android.model.Format
import dev.beam.android.model.TransferUi
import dev.beam.android.model.TransferUi.Phase
import dev.beam.android.service.Notifications
import dev.beam.core.IncomingOffer

@Composable
fun SectionTitle(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = modifier.padding(top = 20.dp, bottom = 8.dp),
    )
}

/** A notice with an optional action: errors, missing permissions. */
@Composable
fun Banner(text: String, action: String? = null, error: Boolean = false, onAction: () -> Unit = {}) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = if (error) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.secondaryContainer,
        ),
        modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
    ) {
        Row(Modifier.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
            if (action != null) TextButton(onClick = onAction) { Text(action) }
        }
    }
}

@Composable
fun TransferCard(t: TransferUi, onCancel: () -> Unit) {
    val context = LocalContext.current
    Card(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        Column(Modifier.padding(16.dp)) {
            Text(
                "${if (t.sending) "To" else "From"} ${t.peerName}",
                style = MaterialTheme.typography.titleMedium,
            )
            Text(
                fileSummary(t),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Spacer(Modifier.padding(4.dp))
            when (t.phase) {
                Phase.ACTIVE -> LinearProgressIndicator(progress = { t.fraction }, modifier = Modifier.fillMaxWidth())
                Phase.WAITING, Phase.INTERRUPTED -> LinearProgressIndicator(Modifier.fillMaxWidth())
                else -> {}
            }
            Text(detail(t), style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 6.dp))
            t.verificationCode?.takeIf { !t.finished }?.let {
                Text(
                    "Code ${Format.code(it)}",
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.padding(top = 4.dp),
                )
            }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                val moving = t.phase == Phase.WAITING || t.phase == Phase.ACTIVE
                if (moving || (t.phase == Phase.INTERRUPTED && t.sending)) {
                    TextButton(onClick = onCancel) { Text("Cancel") }
                }
                if (t.phase == Phase.DONE && t.savedUris.isNotEmpty()) {
                    TextButton(onClick = { openReceived(context, t.savedUris) }) {
                        Text(if (t.savedUris.size == 1) "Open" else "Show in Downloads")
                    }
                }
            }
        }
    }
}

private fun fileSummary(t: TransferUi): String {
    val names = t.fileNames.joinToString()
    val more = t.fileCount - t.fileNames.size
    return if (more > 0) "$names and $more more" else names.ifEmpty { Notifications.files(t.fileCount) }
}

private fun detail(t: TransferUi): String = when (t.phase) {
    Phase.ACTIVE -> buildString {
        append("${Format.bytes(t.doneBytes)} of ${Format.bytes(t.totalBytes)}")
        if (t.bytesPerSecond > 0) append(" · ${Format.speed(t.bytesPerSecond)}")
        t.etaSeconds?.let { append(" · ${Format.duration(it)} left") }
        t.status?.takeIf { it.startsWith("Connection") }?.let { append(" · $it") }
    }
    Phase.WAITING -> t.status ?: "Connecting…"
    Phase.INTERRUPTED -> t.status ?: "Reconnecting…"
    Phase.DONE -> t.status ?: if (t.sending) "Sent ${Format.bytes(t.totalBytes)}" else "Saved to Download/Beam"
    Phase.DECLINED -> "Declined" + (t.status?.let { ": $it" } ?: "")
    Phase.CANCELLED -> "Cancelled" + (t.status?.let { ": $it" } ?: "")
    Phase.FAILED -> "Failed: ${t.status ?: "unknown error"}"
}

private fun openReceived(context: Context, uris: List<String>) {
    val intent = if (uris.size == 1) {
        Intent(Intent.ACTION_VIEW).setData(uris.single().toUri()).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    } else {
        Intent(android.app.DownloadManager.ACTION_VIEW_DOWNLOADS)
    }
    try {
        context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    } catch (_: ActivityNotFoundException) {
        context.startActivity(Intent(android.app.DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}

/** "Laptop wants to send 3 files": compare the code, accept or decline. */
@Composable
fun OfferDialog(offer: IncomingOffer, onAnswer: (accept: Boolean, trust: Boolean) -> Unit) {
    var trust by remember(offer.offerId) { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = {},
        title = { Text("${offer.fromName} wants to send ${Notifications.files(offer.files.size)}") },
        text = {
            Column {
                offer.files.take(5).forEach {
                    Text("${it.name} · ${Format.bytes(it.size.toLong())}", maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (offer.files.size > 5) Text("and ${offer.files.size - 5} more")
                Text(
                    "Total ${Format.bytes(offer.totalBytes.toLong())}",
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(top = 4.dp),
                )
                Text(
                    Format.code(offer.verificationCode),
                    style = MaterialTheme.typography.headlineMedium,
                    fontFamily = FontFamily.Monospace,
                    modifier = Modifier.padding(top = 16.dp),
                )
                Text(
                    if (offer.trusted) "A trusted device." else "Check that ${offer.fromName} shows the same code.",
                    style = MaterialTheme.typography.bodySmall,
                )
                if (!offer.trusted) {
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 8.dp)) {
                        Checkbox(checked = trust, onCheckedChange = { trust = it })
                        Spacer(Modifier.width(4.dp))
                        Text("Trust ${offer.fromName} and accept automatically next time")
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onAnswer(true, trust) }) { Text("Accept") } },
        dismissButton = { TextButton(onClick = { onAnswer(false, false) }) { Text("Decline") } },
    )
}
