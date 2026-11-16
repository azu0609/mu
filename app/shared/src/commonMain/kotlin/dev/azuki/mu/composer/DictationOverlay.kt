package dev.azuki.mu.composer

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.azuki.mu.platform.Dictation
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

@Composable
fun DictationOverlay(visible: Boolean, cancelArmed: Boolean, dictation: Dictation) {
    AnimatedVisibility(
        visible,
        enter = fadeIn() + slideInVertically { it / 3 },
        exit = fadeOut() + slideOutVertically { it / 3 },
    ) {
        val level by dictation.level.collectAsState()
        val partial by dictation.partial.collectAsState()
        val swell by animateFloatAsState(level)
        val colors = MiuixTheme.colorScheme
        val tint by
            animateColorAsState(if (cancelArmed) colors.onSurfaceVariantSummary else colors.primary)
        val ink by animateColorAsState(if (cancelArmed) colors.surface else colors.onPrimary)
        val core = lerp(tint, Color.Black, 0.25f)
        Box(Modifier.fillMaxSize()) {
            Canvas(Modifier.fillMaxSize()) {
                val radius = size.width * (0.62f + 0.08f * swell)
                val center = Offset(size.width / 2, size.height + radius * 0.2f)
                drawCircle(
                    Brush.radialGradient(
                        0f to core,
                        0.35f to tint,
                        0.8f to tint.copy(alpha = 0.85f),
                        1f to tint.copy(alpha = 0f),
                        center = center,
                        radius = radius,
                    ),
                    radius = radius,
                    center = center,
                )
            }
            Column(
                Modifier.align(Alignment.BottomCenter)
                    .fillMaxWidth()
                    .navigationBarsPadding()
                    .padding(horizontal = 32.dp, vertical = 20.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                if (partial.isNotEmpty()) {
                    Text(
                        partial,
                        Modifier.padding(bottom = 24.dp),
                        color = ink,
                        style = MiuixTheme.textStyles.title4,
                        textAlign = TextAlign.Center,
                        maxLines = 4,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                Text(
                    if (cancelArmed) "Release to Cancel" else "Release to Send",
                    color = ink.copy(alpha = 0.85f),
                    style = MiuixTheme.textStyles.body2,
                )
                Text(
                    "Slide upwards to cancel",
                    color = ink.copy(alpha = 0.85f),
                    style = MiuixTheme.textStyles.body2,
                )
            }
        }
    }
}
