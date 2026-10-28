package dev.beam.android

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.core.content.IntentCompat
import androidx.lifecycle.lifecycleScope
import dev.beam.android.service.BeamService
import dev.beam.android.ui.BeamScreen
import dev.beam.android.ui.BeamViewModel
import dev.beam.android.ui.theme.BeamTheme
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val vm: BeamViewModel by viewModels { BeamViewModel.Factory }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // A rotation re-creates the activity with the same intent: stage once.
        if (savedInstanceState == null) handleShare(intent)
        setContent {
            BeamTheme {
                BeamScreen(vm) { lifecycleScope.launch { (application as BeamApp).repository.start() } }
            }
        }
    }

    override fun onStart() {
        super.onStart()
        BeamService.start(this)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleShare(intent)
    }

    /** ACTION_SEND / ACTION_SEND_MULTIPLE from another app's share sheet. */
    private fun handleShare(intent: Intent?) {
        vm.stage(sharedUris(intent).map { it.toString() })
    }

    companion object {
        fun sharedUris(intent: Intent?): List<Uri> = when (intent?.action) {
            Intent.ACTION_SEND -> listOfNotNull(
                IntentCompat.getParcelableExtra(intent, Intent.EXTRA_STREAM, Uri::class.java) ?: intent.clipData?.getItemAt(0)?.uri,
            )
            Intent.ACTION_SEND_MULTIPLE ->
                IntentCompat.getParcelableArrayListExtra(intent, Intent.EXTRA_STREAM, Uri::class.java)
                    ?: intent.clipData?.let { clip -> (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri } }
                    ?: emptyList()
            else -> emptyList()
        }
    }
}
