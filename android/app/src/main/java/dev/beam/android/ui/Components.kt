package dev.beam.android.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Bluetooth
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Laptop
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.PriorityHigh
import androidx.compose.material.icons.outlined.VerifiedUser
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SheetValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.net.toUri
import dev.beam.android.R
import dev.beam.android.model.Format
import dev.beam.android.model.TransferUi
import dev.beam.android.model.TransferUi.Phase
import dev.beam.android.service.Notifications
import dev.beam.android.ui.theme.BeamColors
import dev.beam.android.ui.theme.Ink
import dev.beam.core.DeviceKind
import dev.beam.core.IncomingOffer
import dev.beam.core.NearbyDevice
import dev.beam.core.NearbyLink
import dev.beam.core.Visibility

/** The Beam logo: the mark on an ink square, as on the launcher icon. */
@Composable
fun BeamLogo(size: Dp = 28.dp) {
    Box(
        Modifier.size(size).clip(RoundedCornerShape(size * 0.24f)).background(Ink),
        contentAlignment = Alignment.Center,
    ) {
        Icon(painterResource(R.drawable.ic_notification), null, tint = Color.White, modifier = Modifier.size(size * 0.7f))
    }
}

/** A small heading above a list, with an optional action on the right. */
@Composable
fun SectionLabel(text: String, action: String? = null, onAction: () -> Unit = {}) {
    Row(
        Modifier.fillMaxWidth().padding(start = 4.dp, top = 22.dp, bottom = 6.dp).heightIn(min = 32.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text,
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.SemiBold,
            color = BeamColors.secondaryText,
            modifier = Modifier.weight(1f),
        )
        if (action != null) TextButton(onClick = onAction) { Text(action) }
    }
}

/** A white card holding rows separated by hairlines. */
@Composable
fun ListCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(16.dp)),
        content = content,
    )
}

@Composable
fun RowDivider() = HorizontalDivider(color = BeamColors.divider)

/** A rounded square behind an icon. */
@Composable
fun IconTile(icon: ImageVector, size: Dp = 46.dp) {
    Box(
        Modifier.size(size).clip(RoundedCornerShape(12.dp)).background(BeamColors.tile),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, null, modifier = Modifier.size(22.dp)) }
}

fun deviceIcon(kind: DeviceKind): ImageVector = when (kind) {
    DeviceKind.LAPTOP -> Icons.Filled.Laptop
    DeviceKind.PHONE -> Icons.Filled.PhoneAndroid
    DeviceKind.UNKNOWN -> Icons.Filled.Devices
}

/** "Trusted · No Wi-Fi needed" */
fun deviceDetails(device: NearbyDevice): String =
    listOfNotNull(
        if (device.trusted) "Trusted" else null,
        if (device.link == NearbyLink.BLUETOOTH) "No Wi-Fi needed" else "Same Wi-Fi",
    ).joinToString(" · ")

/** A nearby device. [trailing] is a chevron or a Send button. */
@Composable
fun DeviceRow(device: NearbyDevice, onClick: () -> Unit, trailing: @Composable RowScope.() -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).heightIn(min = 74.dp).padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        IconTile(deviceIcon(device.kind))
        Column(Modifier.weight(1f)) {
            Text(device.name, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (device.link == NearbyLink.BLUETOOTH) {
                    Icon(
                        Icons.Filled.Bluetooth,
                        null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(13.dp).padding(end = 2.dp),
                    )
                }
                Text(deviceDetails(device), fontSize = 13.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        trailing()
    }
}

@Composable
fun Chevron() = Icon(
    Icons.AutoMirrored.Filled.KeyboardArrowRight,
    null,
    tint = MaterialTheme.colorScheme.onSurfaceVariant,
)

/** The one strong action on a screen. */
@Composable
fun PrimaryButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, icon: ImageVector? = null, enabled: Boolean = true) {
    Button(
        onClick = onClick,
        enabled = enabled,
        shape = RoundedCornerShape(14.dp),
        modifier = modifier.heightIn(min = 52.dp),
    ) {
        if (icon != null) {
            Icon(icon, null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(8.dp))
        }
        Text(text, fontSize = 15.sp, fontWeight = FontWeight.Medium)
    }
}

/** A compact filled button for rows: "Send". */
@Composable
fun RowButton(text: String, onClick: () -> Unit) {
    Button(
        onClick = onClick,
        shape = RoundedCornerShape(10.dp),
        contentPadding = ButtonDefaults.ContentPadding,
        modifier = Modifier.height(40.dp),
    ) { Text(text, fontWeight = FontWeight.Medium) }
}

/** A notice with an optional action: errors, missing permissions. */
@Composable
fun Banner(text: String, action: String? = null, error: Boolean = false, onAction: () -> Unit = {}) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(top = 12.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(if (error) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.surfaceContainer)
            .then(
                if (error) Modifier else Modifier.border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(12.dp)),
            )
            .padding(start = 16.dp, end = 6.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text,
            fontSize = 13.5.sp,
            color = if (error) MaterialTheme.colorScheme.onErrorContainer else MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.weight(1f).padding(vertical = 8.dp),
        )
        if (action != null) {
            TextButton(onClick = onAction) {
                Text(action, color = if (error) MaterialTheme.colorScheme.onErrorContainer else MaterialTheme.colorScheme.primary)
            }
        }
    }
}

/** "Visible to everyone · 9 min": tap to change who can see this phone. */
@Composable
fun VisibilityChip(visibility: Visibility, label: String, onChange: (Visibility) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier
                .heightIn(min = 36.dp)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.surfaceContainer)
                .border(1.dp, MaterialTheme.colorScheme.outlineVariant, CircleShape)
                .clickable { open = true }
                .padding(start = 14.dp, end = 10.dp, top = 8.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            VisibilityDot(visibility)
            Text(label, fontSize = 13.sp)
            Icon(Icons.Filled.KeyboardArrowDown, "Change", modifier = Modifier.size(16.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            listOf(
                Triple(Visibility.EVERYONE, "Everyone", "Anyone nearby, for 10 minutes"),
                Triple(Visibility.TRUSTED_ONLY, "Trusted devices", "Only devices you trust"),
                Triple(Visibility.OFF, "Hidden", "Nobody can find or send to you"),
            ).forEach { (value, title, hint) ->
                DropdownMenuItem(
                    text = {
                        Column {
                            Text(title, fontWeight = FontWeight.Medium)
                            Text(hint, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        }
                    },
                    leadingIcon = { VisibilityDot(value) },
                    trailingIcon = { if (value == visibility) Icon(Icons.Filled.Check, "Selected", modifier = Modifier.size(18.dp)) },
                    onClick = {
                        open = false
                        onChange(value)
                    },
                )
            }
        }
    }
}

@Composable
private fun VisibilityDot(visibility: Visibility) {
    val color = when (visibility) {
        Visibility.EVERYONE -> BeamColors.ok
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    Box(
        Modifier
            .size(8.dp)
            .clip(CircleShape)
            .then(if (visibility == Visibility.OFF) Modifier.border(1.5.dp, color, CircleShape) else Modifier.background(color)),
    )
}

/** A pulsing dot: still looking. */
@Composable
fun SearchingDot() {
    val pulse by rememberInfiniteTransition(label = "searching").animateFloat(
        initialValue = 0.25f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(800), RepeatMode.Reverse),
        label = "alpha",
    )
    Box(Modifier.size(10.dp).alpha(pulse).clip(CircleShape).background(MaterialTheme.colorScheme.onSurfaceVariant))
}

/** One transfer: direction or outcome, files, progress, what's happening. */
@Composable
fun TransferRow(t: TransferUi, onCancel: () -> Unit) {
    val context = LocalContext.current
    Row(
        Modifier.fillMaxWidth().heightIn(min = 68.dp).padding(start = 16.dp, end = 4.dp, top = 12.dp, bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        StatusBadge(t)
        Column(Modifier.weight(1f)) {
            Text(fileSummary(t), fontSize = 14.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            when (t.phase) {
                Phase.ACTIVE -> LinearProgressIndicator(
                    progress = { t.fraction },
                    modifier = Modifier.fillMaxWidth().padding(vertical = 7.dp).height(4.dp),
                    trackColor = BeamColors.divider,
                    strokeCap = StrokeCap.Round,
                    drawStopIndicator = {},
                    gapSize = 0.dp,
                )
                Phase.WAITING, Phase.INTERRUPTED -> LinearProgressIndicator(
                    modifier = Modifier.fillMaxWidth().padding(vertical = 7.dp).height(4.dp),
                    trackColor = BeamColors.divider,
                    strokeCap = StrokeCap.Round,
                    gapSize = 0.dp,
                )
                else -> Spacer(Modifier.height(2.dp))
            }
            Text(
                detail(t),
                fontSize = 12.sp,
                color = if (t.phase == Phase.FAILED) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        val moving = t.phase == Phase.WAITING || t.phase == Phase.ACTIVE
        when {
            moving || (t.phase == Phase.INTERRUPTED && t.sending) -> IconButton(onClick = onCancel) {
                Icon(Icons.Filled.Close, "Cancel", tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            t.phase == Phase.DONE && t.savedUris.isNotEmpty() -> TextButton(onClick = { openReceived(context, t.savedUris) }) {
                Text(if (t.savedUris.size == 1) "Open" else "Show")
            }
            else -> Spacer(Modifier.width(12.dp))
        }
    }
}

@Composable
private fun StatusBadge(t: TransferUi) {
    val (icon, background, tint) = when (t.phase) {
        Phase.DONE -> Triple(Icons.Filled.Check, BeamColors.okContainer, BeamColors.ok)
        Phase.FAILED -> Triple(Icons.Filled.PriorityHigh, MaterialTheme.colorScheme.errorContainer, MaterialTheme.colorScheme.error)
        else -> Triple(
            if (t.sending) Icons.Filled.ArrowUpward else Icons.Filled.ArrowDownward,
            BeamColors.tile,
            MaterialTheme.colorScheme.onSurface,
        )
    }
    Box(Modifier.size(32.dp).clip(CircleShape).background(background), contentAlignment = Alignment.Center) {
        Icon(icon, null, tint = tint, modifier = Modifier.size(17.dp))
    }
}

private fun fileSummary(t: TransferUi): String {
    if (t.fileCount > 1 && t.fileNames.all { it.lowercase().endsWith(".jpg") || it.lowercase().endsWith(".png") || it.lowercase().endsWith(".heic") }) {
        return "${t.fileCount} photos"
    }
    val names = t.fileNames.joinToString()
    val more = t.fileCount - t.fileNames.size
    return if (more > 0) "$names and $more more" else names.ifEmpty { Notifications.files(t.fileCount) }
}

private fun detail(t: TransferUi): String {
    val peer = "${if (t.sending) "To" else "From"} ${t.peerName}"
    return when (t.phase) {
        Phase.ACTIVE -> buildString {
            append(peer)
            if (t.bytesPerSecond > 0) append(" · ${Format.speed(t.bytesPerSecond)}")
            t.etaSeconds?.let { append(" · ${Format.duration(it)} left") }
            t.status?.takeIf { it.startsWith("Connection") }?.let { append(" · $it") }
        }
        Phase.WAITING -> buildString {
            append(t.status ?: "$peer · connecting…")
            t.verificationCode?.let { append(" · code ${Format.code(it)}") }
        }
        Phase.INTERRUPTED -> t.status ?: "$peer · reconnecting…"
        Phase.DONE -> "$peer · ${Format.bytes(t.totalBytes)}" + if (t.sending) "" else " · in Download/Beam"
        Phase.DECLINED -> "$peer · declined" + (t.status?.let { ": $it" } ?: "")
        Phase.CANCELLED -> "$peer · cancelled"
        Phase.FAILED -> t.status ?: "Failed"
    }
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
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OfferSheet(offer: IncomingOffer, onAnswer: (accept: Boolean, trust: Boolean) -> Unit) {
    var trust by remember(offer.offerId) { mutableStateOf(false) }
    // Only Accept or Decline closes it.
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true, confirmValueChange = { it != SheetValue.Hidden })
    ModalBottomSheet(onDismissRequest = {}, sheetState = sheet, containerColor = MaterialTheme.colorScheme.surfaceContainer) {
        Column(
            Modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                IconTile(Icons.Filled.Laptop, size = 48.dp)
                Column {
                    Text(
                        "${offer.fromName} wants to send ${Notifications.files(offer.files.size)}",
                        fontSize = 18.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        "${Format.bytes(offer.totalBytes.toLong())} in total",
                        fontSize = 13.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Column(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(12.dp)),
            ) {
                offer.files.take(4).forEachIndexed { i, file ->
                    if (i > 0) RowDivider()
                    Row(Modifier.padding(horizontal = 14.dp, vertical = 12.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(file.name, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                        Text(Format.bytes(file.size.toLong()), fontSize = 14.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                if (offer.files.size > 4) {
                    RowDivider()
                    Text(
                        "and ${offer.files.size - 4} more",
                        fontSize = 13.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp),
                    )
                }
            }
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(MaterialTheme.colorScheme.background)
                    .padding(horizontal = 14.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(
                        if (offer.trusted) "Trusted device · code" else "Check this code matches ${offer.fromName}",
                        fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(Format.code(offer.verificationCode), fontSize = 24.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 3.sp)
                }
                Icon(Icons.Outlined.VerifiedUser, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            if (!offer.trusted) {
                Row(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).clickable { trust = !trust }.heightIn(min = 48.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = trust, onCheckedChange = { trust = it })
                    Text("Always accept from ${offer.fromName}", fontSize = 14.sp)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedButton(
                    onClick = { onAnswer(false, false) },
                    shape = RoundedCornerShape(14.dp),
                    border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
                    modifier = Modifier.weight(1f).heightIn(min = 52.dp),
                ) { Text("Decline", fontSize = 15.sp, fontWeight = FontWeight.Medium) }
                PrimaryButton("Accept", { onAnswer(true, trust) }, Modifier.weight(1f))
            }
        }
    }
}
