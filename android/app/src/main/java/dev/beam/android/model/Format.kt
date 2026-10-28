package dev.beam.android.model

import java.util.Locale
import kotlin.math.roundToInt

/** Human-readable numbers. Decimal units (1 MB = 1,000,000 bytes), like the CLI. */
object Format {
    fun bytes(bytes: Long): String {
        val units = listOf("B", "KB", "MB", "GB", "TB")
        var value = bytes.toDouble()
        var unit = 0
        while (value >= 1000 && unit < units.lastIndex) {
            value /= 1000
            unit++
        }
        return if (unit == 0) "$bytes B" else String.format(Locale.US, if (value < 10) "%.1f %s" else "%.0f %s", value, units[unit])
    }

    fun speed(bytesPerSecond: Double): String = "${bytes(bytesPerSecond.toLong())}/s"

    fun duration(seconds: Double): String {
        if (seconds.isNaN() || seconds < 0) return ""
        val s = seconds.roundToInt().coerceAtLeast(1)
        return when {
            s < 60 -> "$s s"
            s < 3600 -> "${s / 60} min ${s % 60} s"
            else -> "${s / 3600} h ${(s % 3600) / 60} min"
        }
    }

    /** "482193" → "482 193", easier to compare out loud. */
    fun code(code: String): String = if (code.length == 6) "${code.take(3)} ${code.drop(3)}" else code
}
