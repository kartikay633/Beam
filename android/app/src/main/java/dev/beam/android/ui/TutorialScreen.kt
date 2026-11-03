package dev.beam.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Group
import androidx.compose.material.icons.filled.Laptop
import androidx.compose.material.icons.filled.PhoneAndroid
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.SwapHoriz
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch

/** One page of the first-launch tour: plain words, no jargon. */
private data class TourPage(val icons: List<ImageVector>, val title: String, val lines: List<String>)

private val pages = listOf(
    TourPage(
        listOf(Icons.Filled.PhoneAndroid, Icons.Filled.SwapHoriz, Icons.Filled.Laptop),
        "Send files between your phone and computer",
        listOf(
            "Photos, videos, documents: any file, any size.",
            "No cable, no internet and no account. Your files go straight from one device to the other.",
        ),
    ),
    TourPage(
        listOf(Icons.Filled.Laptop),
        "Put Beam on your computer too",
        listOf(
            "Get Beam Share from the Microsoft Store on your Windows computer.",
            "Keep Bluetooth and Wi-Fi switched on on both. They don't need to be connected to anything.",
        ),
    ),
    TourPage(
        listOf(Icons.Filled.CheckCircle),
        "Your devices",
        listOf(
            "Pair a device once and it becomes one of your devices.",
            "Files between your devices arrive straight away, without asking, and you can choose the folder they land in on the computer.",
            "Only pair things that are yours, like your own laptop. Don't pair a friend's or a shared computer.",
        ),
    ),
    TourPage(
        listOf(Icons.Filled.Group),
        "Other people nearby",
        listOf(
            "You can still send to friends without pairing.",
            "Beam asks before anything arrives, and shows a 6-digit code on both screens. If the codes match, it's safe to accept.",
        ),
    ),
    TourPage(
        listOf(Icons.Filled.QrCodeScanner),
        "Pair in 10 seconds",
        listOf(
            "1. On your computer, open Beam and click Your devices. A square code appears.",
            "2. On your phone, tap the scan button at the top of Beam.",
            "3. Point the camera at the code. That's it: you only do this once.",
        ),
    ),
)

/**
 * A short tour for people new to Beam: what it does, what "your devices"
 * means and how to pair. Shown before setup on first launch, with Skip on
 * every page, and again from Settings → How Beam works.
 */
@Composable
fun TutorialScreen(onDone: () -> Unit) {
    val pager = rememberPagerState { pages.size }
    val scope = rememberCoroutineScope()
    val last = pager.currentPage == pages.lastIndex
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding()) {
            Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), horizontalArrangement = Arrangement.End) {
                if (!last) TextButton(onClick = onDone) { Text("Skip") }
            }
            HorizontalPager(pager, Modifier.weight(1f)) { index -> Page(pages[index]) }
            Row(
                Modifier.fillMaxWidth().padding(vertical = 16.dp),
                horizontalArrangement = Arrangement.Center,
            ) {
                repeat(pages.size) { i ->
                    Box(
                        Modifier.padding(horizontal = 4.dp).size(if (i == pager.currentPage) 10.dp else 8.dp).clip(CircleShape)
                            .background(
                                if (i == pager.currentPage) MaterialTheme.colorScheme.onSurface
                                else MaterialTheme.colorScheme.outlineVariant,
                            ),
                    )
                }
            }
            PrimaryButton(
                if (last) "Get started" else "Next",
                onClick = { if (last) onDone() else scope.launch { pager.animateScrollToPage(pager.currentPage + 1) } },
                modifier = Modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, bottom = 16.dp),
            )
        }
    }
}

@Composable
private fun Page(page: TourPage) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 28.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.height(48.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            page.icons.forEach { IconTile(it, size = 72.dp) }
        }
        Text(
            page.title,
            fontSize = 26.sp,
            fontWeight = FontWeight.SemiBold,
            lineHeight = 32.sp,
            modifier = Modifier.padding(top = 32.dp, bottom = 16.dp),
            textAlign = androidx.compose.ui.text.style.TextAlign.Center,
        )
        page.lines.forEach { line ->
            Text(
                line,
                fontSize = 17.sp,
                lineHeight = 25.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
                modifier = Modifier.padding(bottom = 12.dp),
            )
        }
    }
}
