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
    background = Color(0xFFF7F7F8),
    onBackground = Ink,
    surface = Color(0xFFF7F7F8),
    onSurface = Ink,
    surfaceVariant = Color(0xFFF1F2F4),
    onSurfaceVariant = Color(0xFF5F6570),
    surfaceTint = Color.Transparent,
    outline = Color(0xFFC9CCD1),
    outlineVariant = Color(0xFFE4E5E8),
    surfaceBright = Color.White,
    surfaceDim = Color(0xFFE9EAEC),
    surfaceContainerLowest = Color.White,
    surfaceContainerLow = Color.White,
    surfaceContainer = Color.White,
    surfaceContainerHigh = Color(0xFFF1F2F4),
    surfaceContainerHighest = Color(0xFFECEEF1),
    error = Color(0xFFC4321F),
    onError = Color.White,
    errorContainer = Color(0xFFFBE7E4),
    onErrorContainer = Color(0xFF6E1A0F),
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
    background = Color(0xFF0F1114),
    onBackground = Color(0xFFECEDEF),
    surface = Color(0xFF0F1114),
    onSurface = Color(0xFFECEDEF),
    surfaceVariant = Color(0xFF1E2227),
    onSurfaceVariant = Color(0xFF9AA0AA),
    surfaceTint = Color.Transparent,
    outline = Color(0xFF3A3F47),
    outlineVariant = Color(0xFF2A2E35),
    surfaceBright = Color(0xFF23272D),
    surfaceDim = Color(0xFF0F1114),
    surfaceContainerLowest = Color(0xFF0B0D0F),
    surfaceContainerLow = Color(0xFF16191D),
    surfaceContainer = Color(0xFF16191D),
    surfaceContainerHigh = Color(0xFF1E2227),
    surfaceContainerHighest = Color(0xFF23272D),
    error = Color(0xFFF0705E),
    onError = Ink,
    errorContainer = Color(0xFF3A1A15),
    onErrorContainer = Color(0xFFFBD5CF),
)

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
