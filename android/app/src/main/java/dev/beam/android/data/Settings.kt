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

    /** The first-launch setup screen was shown (and answered). */
    var onboarded: Boolean

    /** Folders a PC (by [saveKey]) lets us pick, as it last told us. */
    fun saveFolders(device: String): List<String>
    fun setSaveFolders(device: String, folders: List<String>)

    /** The folder picked for a PC, or null for its default folder. */
    fun saveTo(device: String): String?
    fun setSaveTo(device: String, folder: String?)
}

/** How a PC is remembered for its save folders: by name, the same over Wi-Fi and Bluetooth. */
fun saveKey(deviceName: String) = deviceName.trim().lowercase(java.util.Locale.ROOT)

class MemorySettings(
    override var deviceName: String = "Phone",
    override var visibility: Visibility = Visibility.EVERYONE,
    override var streams: Int = 4,
    override var discovery: DiscoveryChoice = DiscoveryChoice.ANDROID_NSD,
    override var onboarded: Boolean = true,
) : SettingsStore {
    private val folders = mutableMapOf<String, List<String>>()
    private val chosen = mutableMapOf<String, String>()
    override fun saveFolders(device: String) = folders[device].orEmpty()
    override fun setSaveFolders(device: String, folders: List<String>) { this.folders[device] = folders }
    override fun saveTo(device: String) = chosen[device]
    override fun setSaveTo(device: String, folder: String?) {
        if (folder == null) chosen.remove(device) else chosen[device] = folder
    }
}

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

    override var onboarded: Boolean
        get() = prefs.getBoolean("onboarded", false)
        set(value) = prefs.edit { putBoolean("onboarded", value) }

    override fun saveFolders(device: String): List<String> =
        prefs.getString("save_folders:$device", null)?.split(',')?.filter { it.isNotEmpty() }.orEmpty()

    override fun setSaveFolders(device: String, folders: List<String>) =
        prefs.edit { putString("save_folders:$device", folders.joinToString(",")) }

    override fun saveTo(device: String): String? = prefs.getString("save_to:$device", null)

    override fun setSaveTo(device: String, folder: String?) =
        prefs.edit { if (folder == null) remove("save_to:$device") else putString("save_to:$device", folder) }
}
