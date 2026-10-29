package dev.beam.android.ui

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.ui.Alignment
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.beam.android.radio.RadioPermissions

/** Home, and the screens reached from it (each with a back bar). */
enum class Screen(val title: String) {
    HOME("Beam"),
    TRANSFERS("Transfers"),
    SETTINGS("Settings"),
    PAIR("Pair with a laptop"),
}

@Composable
fun BeamScreen(vm: BeamViewModel, onRetry: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val staged by vm.staged.collectAsStateWithLifecycle()
    var screen by rememberSaveable { mutableStateOf(Screen.HOME) }
    val snackbar = remember { SnackbarHostState() }

    // Sharing to Beam lands on Home, where the devices are.
    LaunchedEffect(staged.isNotEmpty()) { if (staged.isNotEmpty()) screen = Screen.HOME }
    LaunchedEffect(state.message) {
        state.message?.let {
            vm.messageShown()
            snackbar.showSnackbar(it)
        }
    }
    BackHandler(enabled = screen != Screen.HOME) { screen = Screen.HOME }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            if (screen != Screen.HOME) {
                Row(
                    Modifier.fillMaxWidth().statusBarsPadding().heightIn(min = 56.dp).padding(start = 4.dp, end = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconButton(onClick = { screen = Screen.HOME }) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back")
                    }
                    Text(screen.title, fontSize = 18.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                    if (screen == Screen.TRANSFERS && state.transfers.any { it.finished }) {
                        TextButton(onClick = vm::clearFinished) { Text("Clear finished") }
                    }
                }
            }
        },
    ) { padding ->
        val modifier = Modifier.padding(padding)
        androidx.compose.foundation.layout.Box(modifier) {
            when (screen) {
                Screen.PAIR -> PairScreen(vm) { screen = Screen.HOME }
                Screen.HOME -> HomeScreen(
                    state = state,
                    staged = staged,
                    vm = vm,
                    notificationBanner = {
                        NotificationPermissionBanner()
                        RadioBanner(vm::refreshRadio)
                    },
                    onRetry = onRetry,
                    onPair = { screen = Screen.PAIR },
                    onSettings = { screen = Screen.SETTINGS },
                    onShowTransfers = { screen = Screen.TRANSFERS },
                )
                Screen.TRANSFERS -> TransfersScreen(state, vm)
                Screen.SETTINGS -> SettingsScreen(state, vm) { screen = Screen.PAIR }
            }
        }
    }

    state.offers.firstOrNull()?.let { offer ->
        OfferSheet(offer) { accept, trust -> vm.respond(offer.offerId, accept, trust) }
    }
}

/**
 * Asks for "Nearby devices" (Bluetooth, and the hotspot) so Beam works with
 * no shared Wi-Fi, and to switch Bluetooth on. Without them Beam still works
 * on a shared network.
 */
@Composable
private fun RadioBanner(onChanged: () -> Unit) {
    val context = LocalContext.current
    if (!RadioPermissions.hasBluetoothLe(context)) return
    var granted by remember { mutableStateOf(RadioPermissions.granted(context)) }
    var bluetoothOn by remember { mutableStateOf(RadioPermissions.bluetoothOn(context)) }
    var denied by rememberSaveable { mutableStateOf(false) }
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
        granted = result.values.all { it }
        denied = !granted
        onChanged()
    }
    val enable = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        bluetoothOn = RadioPermissions.bluetoothOn(context)
        onChanged()
    }
    when {
        !granted && denied -> Banner(
            "Without \"Nearby devices\", Beam only finds devices on your Wi-Fi.",
            action = "Settings",
        ) {
            context.startActivity(
                Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, android.net.Uri.fromParts("package", context.packageName, null)),
            )
        }
        !granted -> Banner("Send without Wi-Fi: Beam uses Bluetooth to find your laptop, then a private Wi-Fi link.", action = "Allow") {
            request.launch(RadioPermissions.required)
        }
        !bluetoothOn -> Banner("Turn on Bluetooth to find devices when you're not on the same Wi-Fi.", action = "Turn on") {
            runCatching { enable.launch(Intent(android.bluetooth.BluetoothAdapter.ACTION_REQUEST_ENABLE)) }
        }
    }
}

/**
 * Asks for notifications (Android 13+), explaining why first. Beam works
 * without them, but incoming offers then only show while the app is open.
 */
@Composable
private fun NotificationPermissionBanner() {
    if (Build.VERSION.SDK_INT < 33) return
    val context = LocalContext.current
    fun granted() = ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
        PackageManager.PERMISSION_GRANTED
    var allowed by remember { mutableStateOf(granted()) }
    var denied by rememberSaveable { mutableStateOf(false) }
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        allowed = it
        denied = !it
    }
    if (allowed) return
    if (denied) {
        Banner(
            "Notifications are off, so you'll only see incoming files while Beam is open.",
            action = "Settings",
        ) {
            context.startActivity(
                Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName),
            )
        }
    } else {
        Banner("Allow notifications to hear about incoming files and see progress.", action = "Allow") {
            request.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
}
