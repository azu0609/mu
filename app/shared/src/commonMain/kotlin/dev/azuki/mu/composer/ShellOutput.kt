package dev.azuki.mu.composer

import androidx.compose.animation.core.animate
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.design.Mono
import dev.azuki.mu.ending
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

// The output of a command run from the composer, peeking out from behind it. Swiping it down
// tucks it away to just its handle; swiping once more from there stops the command and drops it.
@Composable
internal fun ShellOutput(run: ShellRun, onDismiss: () -> Unit) {
    val overlap = 16.dp
    val scroll = rememberScrollState()
    LaunchedEffect(run.output) { scroll.scrollTo(scroll.maxValue) }
    val scope = rememberCoroutineScope()
    val currentOnDismiss by rememberUpdatedState(onDismiss)
    var collapsed by remember { mutableStateOf(false) }
    var delta by remember { mutableFloatStateOf(0f) }
    var height by remember { mutableIntStateOf(0) }
    var settling by remember { mutableStateOf<Job?>(null) }
    val density = LocalDensity.current
    val hidden = with(density) { overlap.toPx() }
    val peek = with(density) { 20.dp.toPx() }
    val threshold = with(density) { 48.dp.toPx() }

    fun gone() = (height - hidden).coerceAtLeast(0f)

    fun rest() = (gone() - peek).coerceAtLeast(0f)

    fun base() = if (collapsed) rest() else 0f

    fun drag(by: Float): Float {
        settling?.cancel()
        val before = base() + delta
        val after = (before + by).coerceIn(0f, if (collapsed) gone() else rest())
        delta = after - base()
        return after - before
    }

    fun settle(velocity: Float) {
        val at = base() + delta
        val away = collapsed && (at > rest() + peek / 2 || velocity > 1_200f && at >= rest())
        val down =
            when {
                away -> true
                velocity > 800f -> true
                velocity < -800f -> false
                collapsed -> at > rest() - threshold
                else -> at > threshold
            }
        collapsed = down
        delta = at - base()
        val target = if (away) gone() - base() else 0f
        settling = scope.launch {
            animate(delta, target, velocity) { value, _ -> delta = value }
            if (away) {
                run.job?.cancel()
                currentOnDismiss()
            }
        }
    }

    fun expand() {
        settling?.cancel()
        delta += base()
        collapsed = false
        settling = scope.launch { animate(delta, 0f) { value, _ -> delta = value } }
    }

    val nested = remember {
        object : NestedScrollConnection {
            override fun onPreScroll(available: Offset, source: NestedScrollSource) =
                if (available.y < 0 && base() + delta > 0f) Offset(0f, drag(available.y))
                else Offset.Zero

            override fun onPostScroll(
                consumed: Offset,
                available: Offset,
                source: NestedScrollSource,
            ) =
                if (available.y > 0 && source == NestedScrollSource.UserInput)
                    Offset(0f, drag(available.y))
                else Offset.Zero

            override suspend fun onPreFling(available: Velocity): Velocity {
                if (base() + delta <= 0f) return Velocity.Zero
                settle(available.y)
                return available
            }
        }
    }
    Box(Modifier.fillMaxWidth().above(-overlap).clipToBounds()) {
        Card(
            modifier =
                Modifier.padding(horizontal = 16.dp)
                    .fillMaxWidth()
                    .onSizeChanged { height = it.height }
                    .graphicsLayer {
                        val at = base() + delta
                        translationY = at
                        val fading = gone() - rest()
                        alpha = if (at > rest() && fading > 0f) 1f - (at - rest()) / fading else 1f
                    }
                    .nestedScroll(nested)
                    .draggable(
                        rememberDraggableState { drag(it) },
                        Orientation.Vertical,
                        onDragStopped = { settle(it) },
                    ),
            cornerRadius = 20.dp,
            onClick = {
                when {
                    collapsed -> expand()
                    run.done -> onDismiss()
                }
            },
        ) {
            Column(
                Modifier.fillMaxWidth()
                    .padding(start = 18.dp, end = 18.dp, top = 8.dp, bottom = overlap + 12.dp)
            ) {
                Box(
                    Modifier.align(Alignment.CenterHorizontally)
                        .size(36.dp, 4.dp)
                        .clip(RoundedCornerShape(2.dp))
                        .background(
                            MiuixTheme.colorScheme.onSurfaceVariantSummary.copy(alpha = 0.3f)
                        )
                )
                Text(
                    buildString {
                        append("$ ").append(run.command)
                        if (run.output.isNotEmpty()) append('\n').append(run.output.trimEnd())
                        run.failure?.let { append('\n').append(it) }
                        ending(run.status)?.let { append("\n[").append(it).append(']') }
                    },
                    Modifier.fillMaxWidth()
                        .padding(top = 10.dp)
                        .heightIn(max = 180.dp)
                        .verticalScroll(scroll),
                    fontFamily = Mono,
                    fontSize = 12.sp,
                    lineHeight = 17.sp,
                    color = MiuixTheme.colorScheme.onSurface,
                )
            }
        }
    }
}
