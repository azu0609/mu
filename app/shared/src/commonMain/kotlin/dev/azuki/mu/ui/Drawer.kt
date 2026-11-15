package dev.azuki.mu.ui

import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.gestures.AnchoredDraggableState
import androidx.compose.foundation.gestures.DraggableAnchors
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.anchoredDraggable
import androidx.compose.foundation.gestures.animateTo
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.BlurEffect
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.nav.gesture.PredictiveBackHandler
import top.yukonga.miuix.kmp.theme.MiuixTheme

internal enum class DrawerValue {
    Closed,
    Open,
}

private val Motion = spring<Float>(dampingRatio = 0.9f, stiffness = 500f)

@Stable
class DrawerState internal constructor(private val scope: CoroutineScope) {
    internal val anchored = AnchoredDraggableState(DrawerValue.Closed)

    val isOpen: Boolean
        get() =
            anchored.currentValue == DrawerValue.Open || anchored.targetValue == DrawerValue.Open

    fun open() = settle(DrawerValue.Open)

    fun close() = settle(DrawerValue.Closed)

    private fun settle(value: DrawerValue) {
        scope.launch { anchored.animateTo(value, Motion) }
    }
}

@Composable
fun rememberDrawerState(): DrawerState {
    val scope = rememberCoroutineScope()
    return remember { DrawerState(scope) }
}

// The drawer's state only moves layers around visually, so input, focus and semantics of whichever
// pane is inactive have to be shut off explicitly here.
@Composable
fun Drawer(
    state: DrawerState,
    sidebar: @Composable (Modifier) -> Unit,
    modifier: Modifier = Modifier,
    blur: () -> Dp = { 0.dp },
    content: @Composable BoxScope.() -> Unit,
) {
    val anchored = state.anchored
    val open = state.isOpen
    val focusManager = LocalFocusManager.current
    LaunchedEffect(open) { focusManager.clearFocus() }
    val sidebarFocus = remember { FocusRequester() }
    val contentFocus = remember { FocusRequester() }
    val surface = MiuixTheme.colorScheme.surface

    BoxWithConstraints(
        modifier.background(surface).anchoredDraggable(anchored, Orientation.Horizontal)
    ) {
        val sidebarWidth = maxWidth * 0.84f
        val width = with(LocalDensity.current) { sidebarWidth.toPx() }
        SideEffect {
            anchored.updateAnchors(
                DraggableAnchors {
                    DrawerValue.Closed at 0f
                    DrawerValue.Open at width
                }
            )
        }
        val offset = { anchored.offset.takeUnless(Float::isNaN) ?: 0f }
        val progress = { (offset() / width).coerceIn(0f, 1f) }

        PredictiveBackHandler(
            enabled = open,
            onProgress = { events ->
                anchored.anchoredDrag { events.collect { dragTo(width * (1f - it.progress)) } }
            },
            onCommit = state::close,
            onCancel = state::open,
        )

        sidebar(
            Modifier.width(sidebarWidth)
                .graphicsLayer {
                    translationX = (offset() - width) * 0.3f
                    alpha = progress()
                }
                .pane(active = open, self = sidebarFocus, fallback = contentFocus)
        )

        Box(
            Modifier.fillMaxSize()
                .graphicsLayer {
                    val progress = progress()
                    translationX = offset()
                    scaleX = 1f - 0.06f * progress
                    scaleY = scaleX
                    transformOrigin = TransformOrigin(0f, 0.5f)
                    shape = RoundedCornerShape(40.dp * progress)
                    clip = progress > 0f
                    shadowElevation = 16.dp.toPx() * progress
                    val blur = maxOf(blur().toPx(), 12.dp.toPx() * progress)
                    renderEffect = if (blur > 0f) BlurEffect(blur, blur, TileMode.Decal) else null
                }
                .background(surface)
                // Compose hit-testing skips layouts without pointer input, so without this, taps on
                // empty content fall through to the sidebar underneath.
                .pointerInput(Unit) { awaitPointerEventScope { while (true) awaitPointerEvent() } }
        ) {
            Box(
                Modifier.fillMaxSize()
                    .pane(active = !open, self = contentFocus, fallback = sidebarFocus),
                content = content,
            )
            if (open) {
                Box(
                    Modifier.fillMaxSize()
                        .pointerInput(Unit) { detectTapGestures { state.close() } }
                        .semantics {
                            contentDescription = "Close sidebar"
                            onClick {
                                state.close()
                                true
                            }
                        }
                )
            }
        }
    }
}

private fun Modifier.pane(active: Boolean, self: FocusRequester, fallback: FocusRequester) =
    then(if (active) Modifier else Modifier.clearAndSetSemantics {})
        .focusRequester(self)
        .focusProperties { onEnter = { if (!active) fallback.requestFocus() } }
        .focusGroup()
