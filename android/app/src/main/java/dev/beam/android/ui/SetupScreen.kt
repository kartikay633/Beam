package dev.beam.android.ui

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bluetooth
import androidx.compose.material.icons.filled.LocationOn
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import com.google.android.gms.common.api.ResolvableApiException
import com.google.android.gms.location.LocationRequest
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.LocationSettingsRequest
import com.google.android.gms.location.Priority
import dev.beam.android.radio.RadioPermissions
import dev.beam.android.radio.RadioSwitch
import dev.beam.android.radio.RadioSwitches

/**
 * First launch: what Beam needs and why, then one tap to allow it all.
 * Shown once; afterwards, missing permissions show as banners on Home.
 */
@Composable
fun SetupScreen(onDone: () -> Unit) {
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { onDone() }
    Column(
        Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().padding(horizontal = 24.dp),
    ) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            Spacer(Modifier.height(40.dp))
            BeamLogo(48.dp)
            Text("Set up Beam", fontSize = 28.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 20.dp))
            Text(
                "Beam sends files straight between your phone and your PC. No internet and no shared Wi-Fi needed.",
                fontSize = 16.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 8.dp, bottom = 28.dp),
            )
            Column(verticalArrangement = Arrangement.spacedBy(22.dp)) {
                Need(Icons.Filled.Bluetooth, "Nearby devices", "Finds your PC and connects to it over Bluetooth.")
                Need(Icons.Filled.Wifi, "Wi-Fi", "A private Wi-Fi link carries the files. You stay on your mobile data.")
                Need(Icons.Filled.LocationOn, "Location", "Some phones need it on to find Wi-Fi devices. Beam never uses your location.")
                Need(Icons.Filled.Notifications, "Notifications", "Shows progress and files arriving.")
            }
            Text(
                "Each time you open Beam, it asks to switch on whatever is off. When you leave, it offers to switch those back off.",
                fontSize = 14.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 28.dp, bottom = 16.dp),
            )
        }
        PrimaryButton(
            "Continue",
            onClick = {
                val wanted = RadioPermissions.required.toList() +
                    if (Build.VERSION.SDK_INT >= 33) listOf(Manifest.permission.POST_NOTIFICATIONS) else emptyList()
                ask.launch(wanted.toTypedArray())
            },
            modifier = Modifier.fillMaxWidth().padding(bottom = 16.dp),
        )
    }
}

@Composable
private fun Need(icon: ImageVector, title: String, why: String) {
    Row(verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        IconTile(icon)
        Column(Modifier.weight(1f)) {
            Text(title, fontSize = 16.sp, fontWeight = FontWeight.Medium)
            Text(why, fontSize = 14.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/**
 * Each time Beam comes to the front, asks (one system dialog each) to turn
 * on Bluetooth, Wi-Fi and Location if they're off, and remembers what it
 * turned on so it can offer to turn those back off later. A switch the user
 * says no to isn't asked about again until Beam is opened afresh.
 */
@Composable
fun SwitchesOnWhenOpened(onChanged: () -> Unit) {
    val context = LocalContext.current
    val queue = remember { mutableStateListOf<RadioSwitch>() }
    val declined = remember { mutableStateListOf<RadioSwitch>() }
    lateinit var askNext: () -> Unit
    val finished = { switch: RadioSwitch ->
        if (RadioSwitches.isOn(context, switch)) RadioSwitches.markTurnedOn(context, switch) else declined += switch
        onChanged()
        askNext()
    }
    val bluetooth = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { finished(RadioSwitch.BLUETOOTH) }
    val wifi = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { finished(RadioSwitch.WIFI) }
    val location = rememberLauncherForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { finished(RadioSwitch.LOCATION) }
    val locationSettings = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { finished(RadioSwitch.LOCATION) }
    askNext = {
        when (val next = queue.removeFirstOrNull()) {
            null -> Unit
            RadioSwitch.BLUETOOTH ->
                // Needs "Nearby devices" first; without it, skip quietly.
                if (RadioPermissions.granted(context)) {
                    runCatching { bluetooth.launch(Intent(android.bluetooth.BluetoothAdapter.ACTION_REQUEST_ENABLE)) }
                        .onFailure { askNext() }
                } else {
                    askNext()
                }
            RadioSwitch.WIFI -> runCatching { wifi.launch(Intent(Settings.Panel.ACTION_WIFI)) }.onFailure { askNext() }
            RadioSwitch.LOCATION -> {
                val activity = context as? Activity
                if (activity == null) {
                    askNext()
                } else {
                    // Google Play services shows a one-tap "Turn on location"
                    // dialog; without it, open the Location settings page.
                    val request = LocationSettingsRequest.Builder()
                        .addLocationRequest(LocationRequest.Builder(Priority.PRIORITY_BALANCED_POWER_ACCURACY, 60_000).build())
                        .setAlwaysShow(true)
                        .build()
                    LocationServices.getSettingsClient(activity).checkLocationSettings(request)
                        .addOnSuccessListener { finished(next) }
                        .addOnFailureListener { e ->
                            if (e is ResolvableApiException) {
                                location.launch(IntentSenderRequest.Builder(e.resolution).build())
                            } else {
                                runCatching { locationSettings.launch(Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS)) }
                                    .onFailure { askNext() }
                            }
                        }
                }
            }
        }
    }
    LifecycleEventEffect(Lifecycle.Event.ON_START) {
        if (queue.isEmpty()) {
            queue += RadioSwitches.off(context).filterNot { it in declined }
            askNext()
        }
    }
}
