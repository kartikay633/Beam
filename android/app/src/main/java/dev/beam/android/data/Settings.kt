package dev.beam.android.data

import android.content.Context
import android.os.Build
import androidx.core.content.edit
import android.provider.Settings as SystemSettings
import dev.beam.core.Visibility

/** Who does mDNS on Android (see DECISIONS.md). */
enum class DiscoveryChoice { ANDROID_NSD, BUILT_IN }

/** What the user can change. Persisted by [PrefsSettings]; in memory in tests. */
interface SettingsStore {
    var deviceName: String
    var visibility: Visibility
    var streams: Int
    var discovery: DiscoveryChoice
}

class MemorySettings(
    override var deviceName: String = "Phone",
    override var visibility: Visibility = Visibility.EVERYONE,
    override var streams: Int = 4,
    override var discovery: DiscoveryChoice = DiscoveryChoice.ANDROID_NSD,
) : SettingsStore

class PrefsSettings(context: Context) : SettingsStore {
    private val prefs = context.getSharedPreferences("settings", Context.MODE_PRIVATE)
    private val defaultName = SystemSettings.Global.getString(context.contentResolver, SystemSettings.Global.DEVICE_NAME)
        ?.takeIf { it.isNotBlank() }
        ?: Build.MODEL

    override var deviceName: String
        get() = prefs.getString("device_name", null) ?: defaultName
        set(value) = prefs.edit { putString("device_name", value.trim().ifEmpty { defaultName }) }

    // "Everyone" is the first-launch default so the phone can be found
    // right away; core drops it to trusted-only after 10 minutes.
    override var visibility: Visibility
        get() = prefs.getString("visibility", null)?.let { runCatching { Visibility.valueOf(it) }.getOrNull() }
            ?: Visibility.EVERYONE
        set(value) = prefs.edit { putString("visibility", value.name) }

    override var streams: Int
        get() = prefs.getInt("streams", 4)
        set(value) = prefs.edit { putInt("streams", value.coerceIn(1, 16)) }

    override var discovery: DiscoveryChoice
        get() = prefs.getString("discovery", null)?.let { runCatching { DiscoveryChoice.valueOf(it) }.getOrNull() }
            ?: DiscoveryChoice.ANDROID_NSD
        set(value) = prefs.edit { putString("discovery", value.name) }
}
