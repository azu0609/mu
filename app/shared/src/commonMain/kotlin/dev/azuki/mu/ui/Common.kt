package dev.azuki.mu.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.dropShadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.shadow.Shadow
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.azuki.mu.model.Status
import dev.azuki.mu.state.Connection
import top.yukonga.miuix.kmp.basic.ButtonDefaults
import top.yukonga.miuix.kmp.basic.DropdownEntry
import top.yukonga.miuix.kmp.basic.DropdownImpl
import top.yukonga.miuix.kmp.basic.FloatingToolbar
import top.yukonga.miuix.kmp.basic.HorizontalDivider
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.IconButton
import top.yukonga.miuix.kmp.basic.ListPopupColumn
import top.yukonga.miuix.kmp.basic.PopupPositionProvider
import top.yukonga.miuix.kmp.basic.SmallTitle
import top.yukonga.miuix.kmp.basic.TextButton
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.window.WindowListPopup

// The shadow of miuix's FloatingToolbar, for surfaces that float but can't be one.
fun Modifier.lifted(shape: Shape) =
    dropShadow(shape, Shadow(radius = 10.dp, color = Color.Black, alpha = 0.1f))

// The round floating buttons the pages are framed with.
@Composable
fun CircleButton(
    icon: ImageVector,
    contentDescription: String?,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    size: Dp = 44.dp,
    tint: Color = MiuixTheme.colorScheme.onSurface,
    enabled: Boolean = true,
) {
    FloatingToolbar(modifier, outSidePadding = PaddingValues(0.dp)) {
        IconButton(onClick = onClick, enabled = enabled, minWidth = size, minHeight = size) {
            Icon(icon, contentDescription, Modifier.size(size * 0.5f), tint = tint)
        }
    }
}

@Composable
fun BackButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    CircleButton(Solar.AltArrowLeftLinear, "Back", onClick, modifier)
}

@Composable
fun CloseButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    CircleButton(Solar.CloseLinear, "Close", onClick, modifier)
}

// A SmallTitle with an action at its end, like the "+" of a section.
@Composable
fun SectionTitle(
    text: String,
    modifier: Modifier = Modifier,
    action: (@Composable () -> Unit)? = null,
) {
    Row(modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        SmallTitle(text, Modifier.weight(1f))
        if (action != null) {
            Box(Modifier.padding(end = 16.dp)) { action() }
        }
    }
}

@Composable
fun SmallIconButton(
    icon: ImageVector,
    contentDescription: String?,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    IconButton(onClick = onClick, modifier = modifier, minWidth = 32.dp, minHeight = 32.dp) {
        Icon(
            icon,
            contentDescription,
            Modifier.size(18.dp),
            tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
        )
    }
}

@Composable
fun Spinning(
    icon: ImageVector,
    modifier: Modifier = Modifier,
    tint: Color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
) {
    val rotation by
        rememberInfiniteTransition()
            .animateFloat(
                initialValue = 0f,
                targetValue = 360f,
                animationSpec =
                    infiniteRepeatable(tween(1100, easing = LinearEasing), RepeatMode.Restart),
            )
    Icon(icon, null, modifier.graphicsLayer { rotationZ = rotation }, tint = tint)
}

@Composable
fun StatusIcon(status: Status, unread: Boolean, modifier: Modifier = Modifier) {
    when {
        status == Status.Running -> Spinning(Solar.LoaderBold, modifier)
        status == Status.Failed ->
            Icon(Solar.DangerTriangleBoldDuotone, "Failed", modifier, tint = Danger)
        status == Status.Finished && unread ->
            Icon(Solar.CheckCircleLineDuotone, "Finished", modifier, tint = Accent)
        else -> Spacer(modifier)
    }
}

fun Connection.label(): String =
    when (this) {
        Connection.Connecting -> "Connecting"
        Connection.Online -> "Online"
        Connection.Offline -> "Offline"
        Connection.Unpaired -> "Unpaired"
    }

// A dropdown anchored to whatever shares its parent Box. Picks in the entries at
// [keepOpen] leave it open, for choosing several.
@Composable
fun Dropdown(
    show: Boolean,
    entries: List<DropdownEntry>,
    onDismiss: () -> Unit,
    alignment: PopupPositionProvider.Align = PopupPositionProvider.Align.Start,
    keepOpen: Set<Int> = emptySet(),
) {
    WindowListPopup(show = show, alignment = alignment, onDismissRequest = onDismiss) {
        val dismiss = LocalDismissState.current
        ListPopupColumn {
            entries.forEachIndexed { entryIndex, entry ->
                entry.items.forEachIndexed { index, item ->
                    key(entryIndex, index) {
                        DropdownImpl(
                            item = item,
                            optionSize = entry.items.size,
                            isSelected = item.selected,
                            index = index,
                            enabled = entry.enabled && item.enabled,
                            isFirst = entryIndex == 0 && index == 0,
                            isLast =
                                entryIndex == entries.lastIndex && index == entry.items.lastIndex,
                            onSelectedIndexChange = {
                                item.onClick?.invoke()
                                if (entryIndex !in keepOpen) dismiss?.invoke()
                            },
                        )
                    }
                }
                if (entryIndex != entries.lastIndex) {
                    HorizontalDivider(
                        Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
                        thickness = 1.5.dp,
                    )
                }
            }
        }
    }
}

@Composable
fun DialogButtons(
    confirm: String,
    onCancel: () -> Unit,
    onConfirm: () -> Unit,
    enabled: Boolean = true,
    destructive: Boolean = false,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        TextButton("Cancel", onClick = onCancel, modifier = Modifier.weight(1f))
        TextButton(
            confirm,
            onClick = onConfirm,
            enabled = enabled,
            modifier = Modifier.weight(1f),
            colors =
                if (destructive) {
                    ButtonDefaults.textButtonColorsPrimary(
                        color = MiuixTheme.colorScheme.error,
                        textColor = MiuixTheme.colorScheme.onError,
                    )
                } else {
                    ButtonDefaults.textButtonColorsPrimary()
                },
        )
    }
}
