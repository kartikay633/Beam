package dev.beam.android.share

import android.content.Context
import android.content.Intent
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import dev.beam.android.MainActivity
import dev.beam.android.R
import dev.beam.core.TrustedDevice

/**
 * Puts trusted devices straight into Android's share sheet ("Direct
 * Share"): share a photo, tap "ThinkPad", and it's on its way, with no app
 * to open and no device to pick.
 */
object DirectShare {
    /** Matches the `<share-target>` in res/xml/shortcuts.xml. */
    const val CATEGORY = "dev.beam.android.category.SEND_TO_DEVICE"
    private const val PREFIX = "device:"

    fun shortcutId(deviceId: String) = PREFIX + deviceId

    /** The device a share-sheet shortcut stands for, if [intent] came from one. */
    fun deviceId(intent: Intent?): String? =
        intent?.getStringExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID)?.removePrefix(PREFIX)?.takeIf { it.isNotEmpty() }

    /** Replaces the shortcuts with one per trusted device. */
    fun publish(context: Context, trusted: List<TrustedDevice>) {
        val max = ShortcutManagerCompat.getMaxShortcutCountPerActivity(context).coerceAtLeast(1)
        val shortcuts = trusted.take(max).mapIndexed { rank, device ->
            ShortcutInfoCompat.Builder(context, shortcutId(device.deviceId))
                .setShortLabel(device.name)
                .setLongLabel("Send to ${device.name}")
                .setIcon(IconCompat.createWithResource(context, R.drawable.ic_shortcut_device))
                .setIntent(
                    Intent(context, MainActivity::class.java)
                        .setAction(Intent.ACTION_VIEW)
                        .putExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID, shortcutId(device.deviceId)),
                )
                .setCategories(setOf(CATEGORY))
                .setLongLived(true)
                .setRank(rank)
                .build()
        }
        runCatching { ShortcutManagerCompat.setDynamicShortcuts(context, shortcuts) }
    }

    /** Ranks a device higher in the share sheet after a send to it. */
    fun used(context: Context, deviceId: String) {
        runCatching { ShortcutManagerCompat.reportShortcutUsed(context, shortcutId(deviceId)) }
    }
}
