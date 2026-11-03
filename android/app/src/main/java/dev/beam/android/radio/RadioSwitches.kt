package dev.beam.android.radio

import android.content.Context
import android.location.LocationManager
import android.net.wifi.WifiManager
import androidx.core.content.edit
import androidx.core.location.LocationManagerCompat

/**
 * The switches Beam needs on: Bluetooth (to find and call devices), Wi-Fi
 * (for the direct link; it needn't be connected to anything) and Location
 * (some phones won't scan for Wi-Fi Direct peers without it).
 *
 * Android only lets an app *ask* to turn these on (one tap each) and never
 * turn them off, so Beam remembers which ones it turned on and, when it
 * stops, offers to turn just those back off (see BeamService).
 */
enum class RadioSwitch(val label: String) { BLUETOOTH("Bluetooth"), WIFI("Wi-Fi"), LOCATION("Location") }

object RadioSwitches {
    private const val PREFS = "radio_switches"
    private const val TURNED_ON = "turned_on"

    fun isOn(context: Context, switch: RadioSwitch): Boolean = when (switch) {
        RadioSwitch.BLUETOOTH -> RadioPermissions.bluetoothOn(context)
        RadioSwitch.WIFI -> context.getSystemService(WifiManager::class.java)?.isWifiEnabled == true
        RadioSwitch.LOCATION -> context.getSystemService(LocationManager::class.java)
            ?.let { LocationManagerCompat.isLocationEnabled(it) } == true
    }

    /** The switches that are off right now, in the order Beam asks for them. */
    fun off(context: Context): List<RadioSwitch> = RadioSwitch.entries.filterNot { isOn(context, it) }

    /** Remembers that Beam turned [switch] on (it was off when Beam opened). */
    fun markTurnedOn(context: Context, switch: RadioSwitch) {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val now = prefs.getStringSet(TURNED_ON, emptySet()).orEmpty() + switch.name
        prefs.edit { putStringSet(TURNED_ON, now) }
    }

    /** What Beam turned on that's still on; clears the list. */
    fun takeTurnedOn(context: Context): List<RadioSwitch> {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val names = prefs.getStringSet(TURNED_ON, emptySet()).orEmpty()
        prefs.edit { remove(TURNED_ON) }
        return RadioSwitch.entries.filter { it.name in names && isOn(context, it) }
    }
}
