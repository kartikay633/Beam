package dev.beam.android.debug

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import dev.beam.android.radio.WifiDirectLink
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Debug builds only: connects to or leaves a laptop over Wi-Fi Direct on
 * command, so the link can be tested on a real phone from a computer:
 *
 *     adb shell am broadcast -n dev.beam.android/.debug.P2pDebugReceiver \
 *         -a dev.beam.android.DEBUG_P2P_CONNECT --es name KARTIKAY
 *     adb shell am broadcast -n dev.beam.android/.debug.P2pDebugReceiver \
 *         -a dev.beam.android.DEBUG_P2P_LEAVE
 *
 * Results go to logcat under the BeamDirect tag.
 */
class P2pDebugReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val app = context.applicationContext
        when (intent.action) {
            CONNECT -> {
                val name = intent.getStringExtra("name")
                val address = intent.getStringExtra("address")
                Log.i(WifiDirectLink.TAG, "debug: connecting to ${name ?: address}")
                scope.launch {
                    runCatching { WifiDirectLink(app).connectToPeer(name, address) }
                        .onSuccess {
                            joined = it
                            Log.i(WifiDirectLink.TAG, "debug: connected, address ${it.address}, laptop ${it.host}")
                        }
                        .onFailure { Log.w(WifiDirectLink.TAG, "debug: connect failed", it) }
                }
            }
            LEAVE -> {
                joined?.release()
                joined = null
                Log.i(WifiDirectLink.TAG, "debug: left")
            }
        }
    }

    companion object {
        const val CONNECT = "dev.beam.android.DEBUG_P2P_CONNECT"
        const val LEAVE = "dev.beam.android.DEBUG_P2P_LEAVE"
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        @Volatile private var joined: WifiDirectLink.Joined? = null
    }
}
