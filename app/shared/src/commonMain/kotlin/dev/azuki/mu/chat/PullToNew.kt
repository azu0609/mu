package dev.azuki.mu.chat

import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.RoundRect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.geometry.center
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

private val IndicatorSize = 44.dp
private val GlyphSize = 22.dp

@Stable
class PullToNew(
    private val scope: CoroutineScope,
    val threshold: Float,
    val travel: Float,
    private val speed: Float,
    private val onArmed: (Boolean) -> Unit,
    private val onRelease: (speed: Float) -> Unit,
) : NestedScrollConnection {
    private var drag by mutableFloatStateOf(0f)

    var flight by mutableFloatStateOf(0f)
        private set

    private var settling: Job? = null
    private var released = false

    var armed by mutableStateOf(false)
        private set

    val offset: Float
        get() = if (drag <= threshold) drag else threshold + (drag - threshold) * 0.5f

    val progress: Float
        get() = (offset / threshold).coerceIn(0f, 1f)

    private fun pull(to: Float) {
        drag = to
        val now = offset >= threshold
        if (now != armed) {
            armed = now
            onArmed(now)
        }
    }

    override fun onPreScroll(available: Offset, source: NestedScrollSource): Offset {
        if (released || source != NestedScrollSource.UserInput) return Offset.Zero
        if (drag <= 0f || available.y <= 0f) return Offset.Zero
        settling?.cancel()
        val used = minOf(available.y, drag)
        pull(drag - used)
        return Offset(0f, used)
    }

    override fun onPostScroll(
        consumed: Offset,
        available: Offset,
        source: NestedScrollSource,
    ): Offset {
        if (released || source != NestedScrollSource.UserInput || available.y >= 0f) {
            return Offset.Zero
        }
        settling?.cancel()
        pull(drag - available.y)
        return Offset(0f, available.y)
    }

    override suspend fun onPreFling(available: Velocity): Velocity {
        if (released || drag <= 0f) return Velocity.Zero
        if (armed) {
            released = true
            val leaving = (-available.y).coerceIn(0f, speed)
            scope.launch {
                animate(0f, travel, leaving, spring(stiffness = 120f)) { value, _ ->
                    flight = value
                }
            }
            onRelease(leaving)
        } else {
            settling = scope.launch {
                animate(drag, 0f, animationSpec = spring(stiffness = 500f)) { value, _ ->
                    pull(value)
                }
            }
        }
        return available
    }
}

@Composable
fun PullIndicator(pull: PullToNew, modifier: Modifier = Modifier) {
    val colors = MiuixTheme.colorScheme
    val fill by
        animateFloatAsState(
            if (pull.armed) 1f else 0f,
            spring(dampingRatio = 0.55f, stiffness = 700f),
        )
    Column(
        modifier.graphicsLayer {
            alpha = pull.progress
            translationY = pull.threshold - pull.offset - pull.flight * 0.5f
        },
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(
            Modifier.size(IndicatorSize)
                .graphicsLayer {
                    val scale = 0.7f + 0.3f * pull.progress
                    scaleX = scale
                    scaleY = scale
                }
                .drawBehind {
                    val stroke = 2.5.dp.toPx()
                    val inset = stroke / 2
                    val ring = Size(size.width - stroke, size.height - stroke)
                    drawCircle(
                        colors.primary.copy(alpha = 0.2f),
                        size.minDimension / 2 - inset,
                        style = Stroke(stroke),
                    )
                    drawArc(
                        colors.primary,
                        startAngle = -90f,
                        sweepAngle = 360f * pull.progress,
                        useCenter = false,
                        topLeft = Offset(inset, inset),
                        size = ring,
                        style = Stroke(stroke, cap = StrokeCap.Round),
                    )
                    drawCircle(colors.primary, size.minDimension / 2 * fill)
                    rotate(-90f * (1f - fill)) {
                        drawPath(
                            plus(center, GlyphSize.toPx() / 24f),
                            colors.onPrimary,
                            alpha = fill.coerceIn(0f, 1f),
                        )
                    }
                }
        )
        Text(
            "New session",
            Modifier.padding(top = 8.dp),
            style = MiuixTheme.textStyles.footnote1,
            color = if (pull.armed) colors.onSurface else colors.onSurfaceVariantSummary,
        )
    }
}

private fun plus(center: Offset, unit: Float) =
    Path().apply {
        val arm = 8f * unit
        val half = 1.75f * unit
        val corner = CornerRadius(half)
        addRoundRect(
            RoundRect(
                Rect(center.x - arm, center.y - half, center.x + arm, center.y + half),
                corner,
            )
        )
        addRoundRect(
            RoundRect(
                Rect(center.x - half, center.y - arm, center.x + half, center.y + arm),
                corner,
            )
        )
    }
