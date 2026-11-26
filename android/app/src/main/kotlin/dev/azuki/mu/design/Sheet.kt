package dev.azuki.mu.design

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.VectorConverter
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.rememberScrollableState
import androidx.compose.foundation.gestures.scrollable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.GenericShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.anim.folmeSpring
import top.yukonga.miuix.kmp.basic.ButtonDefaults
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.basic.TextButton
import top.yukonga.miuix.kmp.nav.gesture.PredictiveBackHandler
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.utils.overScrollVertical
import top.yukonga.miuix.kmp.window.WindowBottomSheet

// A sheet or dialog showing `value`, which stays put until its exit animation ends.
class Presented<T : Any> {
    var value by mutableStateOf<T?>(null)
        private set

    var show by mutableStateOf(false)
        private set

    fun present(value: T) {
        this.value = value
        show = true
    }

    fun dismiss() {
        show = false
    }

    fun finished() {
        if (!show) value = null
    }
}

@Composable
fun SheetClose() {
    val dismiss = LocalDismissState.current
    CloseButton(onClick = { dismiss?.invoke() })
}

// Glides between content heights, yet tracks the room left by the IME as it moves.
private class Glide(private val scope: CoroutineScope) {
    private var room = -1
    private var height = Animatable(0, Int.VectorConverter)

    fun to(target: Int, room: Int): Int {
        if (room != this.room) {
            this.room = room
            height = Animatable(target, Int.VectorConverter)
        } else if (target != height.targetValue) {
            scope.launch { height.animateTo(target, folmeSpring(damping = 0.9f, response = 0.38f)) }
        }
        return height.value
    }
}

private enum class SheetSlot {
    Header,
    Body,
    Footer,
}

@Composable
fun Sheet(
    show: Boolean,
    title: String,
    onDismissRequest: () -> Unit,
    onDismissFinished: (() -> Unit)? = null,
    onBack: (() -> Unit)? = null,
    startAction: @Composable () -> Unit = {
        if (onBack != null) BackButton(onClick = onBack) else SheetClose()
    },
    endAction: @Composable () -> Unit = {},
    bottomAction: (@Composable () -> Unit)? = null,
    content: @Composable (PaddingValues) -> Unit,
) {
    WindowBottomSheet(
        backgroundColor = MiuixTheme.colorScheme.surface,
        show = show,
        onDismissRequest = onDismissRequest,
        onDismissFinished = onDismissFinished,
    ) {
        PredictiveBackHandler(
            enabled = onBack != null,
            onProgress = {},
            onCommit = { onBack?.invoke() },
            onCancel = {},
        )
        val scope = rememberCoroutineScope()
        val glide = remember { Glide(scope) }
        val grip = Modifier.scrollable(rememberScrollableState { 0f }, Orientation.Vertical)
        SubcomposeLayout(
            // miuix keeps an empty title row's padding even without a title, which this takes
            // over so the body can scroll right up to the drag handle.
            Modifier.layout { measurable, constraints ->
                val row = 18.dp.roundToPx()
                val placeable =
                    measurable.measure(
                        if (constraints.hasBoundedHeight)
                            constraints.copy(maxHeight = constraints.maxHeight + row)
                        else constraints
                    )
                layout(placeable.width, (placeable.height - row).coerceAtLeast(0)) {
                    placeable.place(0, -row)
                }
            }
        ) { constraints ->
            val loose = constraints.copy(minWidth = 0, minHeight = 0)
            val header =
                subcompose(SheetSlot.Header) {
                        Box(
                            Modifier.fillMaxWidth()
                                .then(grip)
                                .scrim(edgeToEdge = true)
                                .padding(top = 6.dp, bottom = 12.dp)
                        ) {
                            Box(Modifier.align(Alignment.CenterStart)) { startAction() }
                            Text(
                                title,
                                Modifier.align(Alignment.Center).padding(horizontal = 52.dp),
                                fontSize = MiuixTheme.textStyles.title4.fontSize,
                                fontWeight = FontWeight.Medium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Box(Modifier.align(Alignment.CenterEnd)) { endAction() }
                        }
                    }
                    .map { it.measure(loose) }
            val footer =
                subcompose(SheetSlot.Footer) {
                        Box(
                            Modifier.fillMaxWidth()
                                .then(grip)
                                .then(
                                    if (bottomAction != null)
                                        Modifier.scrim(top = false).padding(top = 16.dp)
                                    else Modifier
                                )
                                .navigationBarsPadding()
                                .padding(bottom = 16.dp)
                        ) {
                            bottomAction?.invoke()
                        }
                    }
                    .map { it.measure(loose) }
            val top = header.maxOfOrNull { it.height } ?: 0
            val bottom = footer.maxOfOrNull { it.height } ?: 0
            val padding = PaddingValues(top = top.toDp(), bottom = bottom.toDp())
            val body = subcompose(SheetSlot.Body) { content(padding) }.map { it.measure(loose) }
            val target =
                maxOf(body.maxOfOrNull { it.height } ?: 0, top + bottom)
                    .coerceIn(constraints.minHeight, constraints.maxHeight)
            val height = glide.to(target, constraints.maxHeight)
            layout(constraints.maxWidth, height) {
                body.forEach {
                    it.placeWithLayer(0, 0) {
                        clip = it.height > height
                        shape = GenericShape { size, _ ->
                            addRect(Rect(0f, 0f, size.width, height.toFloat()))
                        }
                    }
                }
                header.forEach { it.place(0, 0) }
                footer.forEach { it.place(0, height - it.height) }
            }
        }
    }
}

@Composable
fun SheetColumn(
    padding: PaddingValues,
    verticalArrangement: Arrangement.Vertical = Arrangement.spacedBy(12.dp),
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        Modifier.overScrollVertical().verticalScroll(rememberScrollState()).padding(padding),
        verticalArrangement = verticalArrangement,
        content = content,
    )
}

@Composable
fun PrimaryButton(text: String, enabled: Boolean = true, onClick: () -> Unit) {
    TextButton(
        text,
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier.fillMaxWidth(),
        cornerRadius = 24.dp,
        colors = ButtonDefaults.textButtonColorsPrimary(),
    )
}

@Composable
fun Note(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        modifier.padding(horizontal = 12.dp),
        style = MiuixTheme.textStyles.footnote1,
        color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
    )
}
