package dev.beam.android.service

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import dev.beam.android.MainActivity
import dev.beam.android.R
import dev.beam.android.data.BeamState
import dev.beam.android.model.Format
import dev.beam.android.model.TransferUi
import dev.beam.core.IncomingOffer
import dev.beam.core.Visibility

object Notifications {
    const val CHANNEL_STATUS = "status"
    const val CHANNEL_OFFERS = "offers"
    const val CHANNEL_DONE = "done"
    const val ID_STATUS = 1

    fun createChannels(context: Context) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannels(
            listOf(
                NotificationChannel(CHANNEL_STATUS, "Beam is on", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Shown while Beam can receive, with transfer progress"
                },
                NotificationChannel(CHANNEL_OFFERS, "Incoming files", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "Someone wants to send you files"
                },
                NotificationChannel(CHANNEL_DONE, "Finished transfers", NotificationManager.IMPORTANCE_DEFAULT),
            ),
        )
    }

    private fun openApp(context: Context): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun serviceAction(context: Context, action: String, id: String?, request: Int): PendingIntent =
        PendingIntent.getService(
            context,
            request,
            Intent(context, BeamService::class.java).setAction(action).putExtra(BeamService.EXTRA_ID, id),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

    /** The ongoing notification: progress while busy, otherwise who can see us. */
    fun status(context: Context, state: BeamState, nowMs: Long) = NotificationCompat.Builder(context, CHANNEL_STATUS)
        .setSmallIcon(R.drawable.ic_notification)
        .setOngoing(true)
        .setOnlyAlertOnce(true)
        .setContentIntent(openApp(context))
        .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        .apply {
            val busy = state.busy
            if (busy.isEmpty()) {
                setContentTitle("Beam is on")
                setContentText(visibilityText(state, nowMs))
                addAction(0, "Stop", serviceAction(context, BeamService.ACTION_STOP, null, 1))
                return@apply
            }
            val first = busy.first()
            val done = busy.sumOf { it.doneBytes }
            val total = busy.sumOf { it.totalBytes }
            setContentTitle(
                if (busy.size > 1) "${busy.size} transfers"
                else "${if (first.sending) "Sending to" else "Receiving from"} ${first.peerName}",
            )
            setContentText(progressText(first, busy.size, done, total))
            if (total > 0 && first.phase == TransferUi.Phase.ACTIVE) {
                setProgress(1000, (done * 1000 / total).toInt(), false)
            } else {
                setProgress(0, 0, true)
            }
            addAction(0, "Cancel", serviceAction(context, BeamService.ACTION_CANCEL, first.id, first.id.hashCode()))
        }
        .build()

    fun visibilityText(state: BeamState, nowMs: Long): String = when (state.visibility) {
        Visibility.EVERYONE -> state.everyoneUntilMs
            ?.let { "Visible to everyone for ${((it - nowMs) / 60_000 + 1).coerceAtLeast(1)} min" }
            ?: "Visible to everyone"
        Visibility.TRUSTED_ONLY -> "Visible to your trusted devices"
        Visibility.OFF -> "Hidden; you can still send"
    }

    private fun progressText(t: TransferUi, count: Int, done: Long, total: Long): String = when {
        t.phase == TransferUi.Phase.WAITING -> t.status ?: "Connecting…"
        t.phase == TransferUi.Phase.INTERRUPTED -> t.status ?: "Reconnecting…"
        count > 1 -> "${Format.bytes(done)} of ${Format.bytes(total)}"
        else -> buildString {
            append("${Format.bytes(done)} of ${Format.bytes(total)}")
            if (t.bytesPerSecond > 0) append(" · ${Format.speed(t.bytesPerSecond)}")
            t.etaSeconds?.let { append(" · ${Format.duration(it)} left") }
        }
    }

    fun offer(context: Context, offer: IncomingOffer) = NotificationCompat.Builder(context, CHANNEL_OFFERS)
        .setSmallIcon(R.drawable.ic_notification)
        .setContentTitle("${offer.fromName} wants to send ${files(offer.files.size)}")
        .setContentText("${Format.bytes(offer.totalBytes.toLong())} · code ${Format.code(offer.verificationCode)}")
        .setPriority(NotificationCompat.PRIORITY_HIGH)
        .setCategory(NotificationCompat.CATEGORY_CALL)
        .setContentIntent(openApp(context))
        .setTimeoutAfter(offer.timeoutSecs.toLong() * 1000)
        .setAutoCancel(true)
        .addAction(0, "Decline", serviceAction(context, BeamService.ACTION_DECLINE, offer.offerId, offer.offerId.hashCode() + 1))
        .addAction(0, "Accept", serviceAction(context, BeamService.ACTION_ACCEPT, offer.offerId, offer.offerId.hashCode() + 2))
        .build()

    fun finished(context: Context, t: TransferUi) = NotificationCompat.Builder(context, CHANNEL_DONE)
        .setSmallIcon(R.drawable.ic_notification)
        .setContentTitle(
            when (t.phase) {
                TransferUi.Phase.DONE -> if (t.sending) "Sent ${files(t.fileCount)} to ${t.peerName}"
                else "Received ${files(t.fileCount)} from ${t.peerName}"
                TransferUi.Phase.DECLINED -> "${t.peerName} declined"
                TransferUi.Phase.CANCELLED -> "Transfer cancelled"
                else -> "Transfer with ${t.peerName} failed"
            },
        )
        .setContentText(t.status ?: if (t.sending) t.fileNames.joinToString() else "Saved to Download/Beam")
        .setContentIntent(openApp(context))
        .setAutoCancel(true)
        .build()

    fun files(count: Int) = if (count == 1) "1 file" else "$count files"
}
