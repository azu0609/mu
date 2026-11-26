package dev.azuki.mu.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import top.yukonga.miuix.kmp.theme.ColorSchemeMode
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.theme.ThemeController

private val Seed = Color(0xFF00C3D0)

val Mono = FontFamily.Monospace

// Colors follow the wallpaper where the system derives a palette from it, and `Seed` elsewhere.
@Composable
fun MuTheme(wallpaperColors: Boolean, content: @Composable () -> Unit) {
    val controller = remember {
        ThemeController(
            ColorSchemeMode.MonetSystem,
            keyColor = if (wallpaperColors) null else Seed,
        )
    }
    MiuixTheme(controller = controller, content = content)
}
