package dev.beam.android.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight

// Neutral greys with ink, the logo's colour, as the only accent; the same
// palette as the desktop app (desktop/ui/src/styles.css). Every Material
// role is set: the defaults are purple, and dynamic colour follows the
// wallpaper, so neither is used.
val Ink = Color(0xFF14171C)

private val Light = lightColorScheme(
    primary = Ink,
    onPrimary = Color.White,
    primaryContainer = Color(0xFFECEEF1),
    onPrimaryContainer = Ink,
    inversePrimary = Color(0xFFECEDEF),
    secondary = Color(0xFF3B4048),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFE4E6EA),
    onSecondaryContainer = Ink,
    tertiary = Color(0xFF3B4048),
    onTertiary = Color.White,
    tertiaryContainer = Color(0xFFE4E6EA),
    onTertiaryContainer = Ink,
    background = Color(0xFFF4F5F7),
    onBackground = Ink,
    surface = Color(0xFFF4F5F7),
    onSurface = Ink,
    surfaceVariant = Color(0xFFF1F2F4),
    onSurfaceVariant = Color(0xFF5F6672),
    surfaceTint = Color.Transparent,
    outline = Color(0xFFC9CCD1),
    outlineVariant = Color(0xFFE4E6EA),
    surfaceBright = Color.White,
    surfaceDim = Color(0xFFE9EAEC),
    surfaceContainerLowest = Color.White,
    surfaceContainerLow = Color.White,
    surfaceContainer = Color.White,
    surfaceContainerHigh = Color(0xFFF1F2F4),
    surfaceContainerHighest = Color(0xFFECEEF1),
    error = Color(0xFFB42318),
    onError = Color.White,
    errorContainer = Color(0xFFFCEDEB),
    onErrorContainer = Color(0xFFB42318),
)

private val Dark = darkColorScheme(
    primary = Color(0xFFECEDEF),
    onPrimary = Ink,
    primaryContainer = Color(0xFF23272D),
    onPrimaryContainer = Color(0xFFECEDEF),
    inversePrimary = Ink,
    secondary = Color(0xFFC3C7CE),
    onSecondary = Ink,
    secondaryContainer = Color(0xFF2A2E35),
    onSecondaryContainer = Color(0xFFECEDEF),
    tertiary = Color(0xFFC3C7CE),
    onTertiary = Ink,
    tertiaryContainer = Color(0xFF2A2E35),
    onTertiaryContainer = Color(0xFFECEDEF),
    background = Color(0xFF111317),
    onBackground = Color(0xFFECEDEF),
    surface = Color(0xFF111317),
    onSurface = Color(0xFFECEDEF),
    surfaceVariant = Color(0xFF1E2227),
    onSurfaceVariant = Color(0xFF9AA0AA),
    surfaceTint = Color.Transparent,
    outline = Color(0xFF3A3F47),
    outlineVariant = Color(0xFF2A2E35),
    surfaceBright = Color(0xFF23272D),
    surfaceDim = Color(0xFF0F1114),
    surfaceContainerLowest = Color(0xFF0B0D0F),
    surfaceContainerLow = Color(0xFF1A1D22),
    surfaceContainer = Color(0xFF1A1D22),
    surfaceContainerHigh = Color(0xFF1E2227),
    surfaceContainerHighest = Color(0xFF23272D),
    error = Color(0xFFF0857A),
    onError = Ink,
    errorContainer = Color(0xFF3A1D1A),
    onErrorContainer = Color(0xFFF0857A),
)

/** Colours Material has no role for: success, and the tile behind icons. */
object BeamColors {
    val ok: Color @Composable get() = if (isSystemInDarkTheme()) Color(0xFF5CC28C) else Color(0xFF1F7A4D)
    val okContainer: Color @Composable get() = if (isSystemInDarkTheme()) Color(0xFF173226) else Color(0xFFE7F3EC)
    val tile: Color @Composable get() = if (isSystemInDarkTheme()) Color(0xFF24282F) else Color(0xFFF1F2F4)
    val divider: Color @Composable get() = if (isSystemInDarkTheme()) Color(0xFF24272D) else Color(0xFFEEF0F2)
    val secondaryText: Color @Composable get() = if (isSystemInDarkTheme()) Color(0xFFC9CDD4) else Color(0xFF3A404B)
}

private val BeamTypography = Typography().run {
    copy(
        headlineSmall = headlineSmall.copy(fontWeight = FontWeight.SemiBold),
        titleLarge = titleLarge.copy(fontWeight = FontWeight.SemiBold),
        titleMedium = titleMedium.copy(fontWeight = FontWeight.SemiBold),
        titleSmall = titleSmall.copy(fontWeight = FontWeight.SemiBold),
    )
}

@Composable
fun BeamTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = if (isSystemInDarkTheme()) Dark else Light,
        typography = BeamTypography,
        content = content,
    )
}
