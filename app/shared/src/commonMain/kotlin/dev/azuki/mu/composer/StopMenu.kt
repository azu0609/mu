package dev.azuki.mu.composer

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.azuki.mu.design.Glyphs
import dev.azuki.mu.design.Solar
import kotlin.math.roundToInt
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

enum class StopChoice(val label: String, val children: Boolean) {
    WithSubagents("Stop with subagents", true),
    Session("Stop", false),
}

// Top to bottom, as the menu stacks them above the stop button.
private val Choices = StopChoice.entries

private val Item = 56.dp
private val Cancel = 40.dp
private val Gap = 16.dp
private val Pill = 48.dp

// Holding the stop button opens it over the screen; the finger picks by sliding up the stack and
// lifting, and lifting on the cancel button below it, where it was held, picks nothing.
class StopMenu {
    var open by mutableStateOf(false)
        internal set

    // The middle of the stop button, in the window.
    internal var anchor by mutableStateOf(Offset.Zero)
    internal var choice by mutableStateOf<StopChoice?>(null)
}

@Composable
internal fun StopButton(menu: StopMenu, onStop: (children: Boolean) -> Unit) {
    val haptics = LocalHapticFeedback.current
    val stop by rememberUpdatedState(onStop)
    Box(
        Modifier.size(36.dp)
            .clip(CircleShape)
            .background(MiuixTheme.colorScheme.primary)
            .onGloballyPositioned { menu.anchor = it.boundsInWindow().center }
            .semantics {
                role = Role.Button
                contentDescription = "Stop"
                onClick {
                    stop(false)
                    true
                }
            }
            .pointerInput(menu) {
                awaitEachGesture {
                    val down = awaitFirstDown()
                    down.consume()
                    var up: PointerInputChange? = null
                    val tapped =
                        withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
                            up = waitForUpOrCancellation()
                        }
                    if (tapped != null) {
                        up?.let {
                            it.consume()
                            stop(false)
                        }
                        return@awaitEachGesture
                    }
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    menu.choice = null
                    menu.open = true
                    val floor = Cancel.toPx() / 2 + Gap.toPx()
                    val item = Item.toPx()
                    while (true) {
                        val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id }
                        if (change == null) {
                            menu.choice = null
                            break
                        }
                        change.consume()
                        if (!change.pressed) break
                        val rise = size.height / 2f - change.position.y - floor
                        val choice =
                            if (rise < 0) null
                            else
                                Choices[
                                    (Choices.lastIndex - (rise / item).toInt()).coerceAtLeast(0)]
                        if (choice != menu.choice) {
                            menu.choice = choice
                            haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                        }
                    }
                    menu.open = false
                    menu.choice?.let { stop(it.children) }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Icon(Glyphs.Stop, null, Modifier.size(20.dp), tint = MiuixTheme.colorScheme.onPrimary)
    }
}

@Composable
fun StopOverlay(menu: StopMenu) {
    AnimatedVisibility(menu.open, enter = fadeIn(), exit = fadeOut()) {
        val colors = MiuixTheme.colorScheme
        var origin by remember { mutableStateOf(Offset.Zero) }
        val center = menu.anchor - origin
        val (item, gap, pill, cancel) =
            with(LocalDensity.current) { listOf(Item, Gap, Pill, Cancel).map { it.toPx() } }
        val bottom = center.y - cancel / 2 - gap
        val top = bottom - item * Choices.size
        Box(
            Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.3f)).onGloballyPositioned {
                origin = it.positionInWindow()
            }
        ) {
            Box(
                Modifier.at(center.x, (top + bottom) / 2)
                    .size(Pill, Item * Choices.size)
                    .clip(RoundedCornerShape(Pill / 2))
                    .background(colors.surfaceContainer)
            )
            Choices.forEachIndexed { index, choice ->
                val y = top + item * (index + 0.5f)
                val picked = menu.choice == choice
                val dot by animateDpAsState(if (picked) 14.dp else 10.dp)
                Box(
                    Modifier.at(center.x, y)
                        .size(dot)
                        .clip(CircleShape)
                        .background(if (picked) colors.primary else colors.onSurfaceVariantSummary)
                )
                Text(
                    choice.label,
                    Modifier.at(center.x - pill / 2 - gap, y, end = true),
                    color = if (picked) colors.onSurface else colors.onSurfaceVariantSummary,
                    style = MiuixTheme.textStyles.title4,
                )
            }
            Box(
                Modifier.at(center.x, center.y)
                    .size(Cancel)
                    .clip(CircleShape)
                    .background(
                        if (menu.choice == null) colors.onSurfaceVariantSummary
                        else colors.surfaceContainer
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Solar.CloseBold,
                    "Cancel",
                    Modifier.size(20.dp),
                    tint = if (menu.choice == null) colors.surface else colors.onSurface,
                )
            }
        }
    }
}

// Centers what it holds on (x, y), or ends it at x.
private fun Modifier.at(x: Float, y: Float, end: Boolean = false) =
    layout { measurable, constraints ->
        val placeable = measurable.measure(constraints.copy(minWidth = 0, minHeight = 0))
        layout(placeable.width, placeable.height) {
            val left = if (end) x - placeable.width else x - placeable.width / 2f
            placeable.place(left.roundToInt(), (y - placeable.height / 2f).roundToInt())
        }
    }
