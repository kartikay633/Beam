package dev.beam.android

import android.content.Intent
import android.net.Uri
import androidx.compose.ui.test.junit4.v2.createEmptyComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Sharing files to Beam from another app stages them on Home, ready for a device. */
@RunWith(AndroidJUnit4::class)
class ShareIntentTest {
    @get:Rule
    val compose = createEmptyComposeRule()

    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    private fun file(name: String, bytes: Int): Uri {
        val file = File(context.cacheDir, name)
        file.writeBytes(ByteArray(bytes) { it.toByte() })
        return Uri.fromFile(file)
    }

    private fun share(intent: Intent) =
        ActivityScenario.launch<MainActivity>(intent.setClass(context, MainActivity::class.java))

    @Test
    fun oneSharedFileIsReadyToSend() {
        val uri = file("holiday.jpg", 2_000)
        share(Intent(Intent.ACTION_SEND).setType("image/jpeg").putExtra(Intent.EXTRA_STREAM, uri)).use {
            compose.waitUntil(10_000) { compose.onAllNodes(hasTextExactly("holiday.jpg")).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("Ready to send").assertExists()
        }
    }

    @Test
    fun severalSharedFilesAreAllStaged() {
        val uris = arrayListOf(file("a.txt", 10), file("b.txt", 20))
        share(Intent(Intent.ACTION_SEND_MULTIPLE).setType("*/*").putParcelableArrayListExtra(Intent.EXTRA_STREAM, uris)).use {
            compose.waitUntil(10_000) { compose.onAllNodes(hasTextExactly("b.txt")).fetchSemanticsNodes().isNotEmpty() }
            compose.onNodeWithText("a.txt").assertExists()
            compose.onNodeWithText("2 files", substring = true).assertExists()
        }
    }

    @Test
    fun sharedUrisAreReadFromBothExtrasAndClipData() {
        val uri = file("c.txt", 1)
        val viaClip = Intent(Intent.ACTION_SEND).apply { clipData = android.content.ClipData.newRawUri("", uri) }
        assertEquals(listOf(uri), MainActivity.sharedUris(viaClip))
        assertEquals(emptyList<Uri>(), MainActivity.sharedUris(Intent(Intent.ACTION_VIEW)))
    }

    private fun hasTextExactly(text: String) = androidx.compose.ui.test.hasText(text, substring = false)
}
