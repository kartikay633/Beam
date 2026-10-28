package dev.beam.android.ui

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Devices
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.SwapVert
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle

enum class Tab(val label: String, val icon: ImageVector) {
    HOME("Home", Icons.Filled.Home),
    TRANSFERS("Transfers", Icons.Filled.SwapVert),
    DEVICES("Devices", Icons.Filled.Devices),
    SETTINGS("Settings", Icons.Filled.Settings),
}

@Composable
fun BeamScreen(vm: BeamViewModel, onRetry: () -> Unit) {
    val state by vm.state.collectAsStateWithLifecycle()
    val staged by vm.staged.collectAsStateWithLifecycle()
    var tab by rememberSaveable { mutableStateOf(Tab.HOME) }
    var pairing by rememberSaveable { mutableStateOf(false) }
    val snackbar = remember { SnackbarHostState() }

    // Sharing to Beam lands on Home, where the devices are.
    LaunchedEffect(staged.isNotEmpty()) { if (staged.isNotEmpty()) tab = Tab.HOME }
    LaunchedEffect(state.message) {
        state.message?.let {
            vm.messageShown()
            snackbar.showSnackbar(it)
        }
    }
    BackHandler(enabled = pairing || tab != Tab.HOME) {
        if (pairing) pairing = false else tab = Tab.HOME
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = {
            if (!pairing) {
                NavigationBar {
                    Tab.entries.forEach {
                        NavigationBarItem(
                            selected = tab == it,
                            onClick = { tab = it },
                            icon = { Icon(it.icon, null) },
                            label = { Text(it.label) },
                        )
                    }
                }
            }
        },
    ) { padding ->
        val modifier = Modifier.padding(padding)
        androidx.compose.foundation.layout.Box(modifier) {
            when {
                pairing -> PairScreen(vm) { pairing = false }
                tab == Tab.HOME -> HomeScreen(
                    state = state,
                    staged = staged,
                    vm = vm,
                    notificationBanner = { NotificationPermissionBanner() },
                    onRetry = onRetry,
                    onPair = { pairing = true },
                    onShowTransfers = { tab = Tab.TRANSFERS },
                )
                tab == Tab.TRANSFERS -> TransfersScreen(state, vm)
                tab == Tab.DEVICES -> TrustedScreen(state, vm) { pairing = true }
                else -> SettingsScreen(state, vm)
            }
        }
    }

    state.offers.firstOrNull()?.let { offer ->
        OfferDialog(offer) { accept, trust -> vm.respond(offer.offerId, accept, trust) }
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
