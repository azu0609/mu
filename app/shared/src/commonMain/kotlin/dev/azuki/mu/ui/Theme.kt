package dev.azuki.mu.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import top.yukonga.miuix.kmp.theme.ColorSchemeMode
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.theme.ThemeController
import top.yukonga.miuix.kmp.theme.darkColorScheme
import top.yukonga.miuix.kmp.theme.lightColorScheme

val Accent = Color(0xFF00C3D0)
val Danger = Color(0xFFFF4D4F)

val Mono = FontFamily.Monospace

@Composable
fun MuTheme(content: @Composable () -> Unit) {
    val controller = remember {
        ThemeController(
            ColorSchemeMode.System,
            lightColors =
                lightColorScheme(
                    primary = Accent,
                    primaryVariant = Accent,
                    onPrimaryVariant = Color(0xFFA6EBEF),
                    disabledPrimary = Color(0xFFB3EDF1),
                    disabledOnPrimary = Color(0xFFF2FCFD),
                    disabledPrimaryButton = Color(0xFF7FDDE3),
                    disabledPrimarySlider = Color(0xFFA9E3E7),
                    primaryContainer = Color(0xFF33CFD9),
                    tertiaryContainer = Color(0xFFE5F9FA),
                    onTertiaryContainer = Accent,
                    tertiaryContainerVariant = Color(0xFFE5F9FA),
                    sliderKeyPointForeground = Color(0xFF5ED8E0),
                ),
            darkColors =
                darkColorScheme(
                    primary = Accent,
                    primaryVariant = Color(0xFF00A9B4),
                    onPrimaryVariant = Color(0xFF8CD9DE),
                    disabledPrimary = Color(0xFF1D4A4D),
                    disabledOnPrimary = Color(0xFF678C8F),
                    disabledPrimaryButton = Color(0xFF1D4A4D),
                    disabledOnPrimaryButton = Color(0xFF678C8F),
                    disabledPrimarySlider = Color(0xFF2F5F62),
                    primaryContainer = Color(0xFF00A9B4),
                    tertiaryContainer = Color(0xFF173A3D),
                    onTertiaryContainer = Accent,
                    sliderKeyPointForeground = Color(0xFF5ED8E0),
                ),
        )
    }
    MiuixTheme(controller = controller, content = content)
}
