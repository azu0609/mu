package dev.azuki.mu.composer

import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import dev.azuki.mu.Mu
import dev.azuki.mu.api.MuException
import dev.azuki.mu.chat.Destination
import dev.azuki.mu.design.Accent
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.icon
import dev.azuki.mu.platform.Attachment
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.CardDefaults
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.PopupPositionProvider
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.window.WindowListPopup

@Composable
internal fun AttachButton(state: ComposerState, mu: Mu, destination: Destination) {
    var open by remember { mutableStateOf(false) }
    val size = with(LocalDensity.current) { TileSize.roundToPx() }

    // Picking swaps the dictation bar for the keyboard's, which would cancel this button's scope.
    fun pick(source: suspend () -> List<Attachment>) {
        val host = destination.host ?: return mu.toast("Add a host first")
        mu.launch {
            val picked = source()
            if (picked.isEmpty()) return@launch
            state.useKeyboard(true)
            state.uploading++
            try {
                picked.forEach { attachment ->
                    try {
                        val path = mu.remote(host).upload(attachment.name, attachment.data)
                        if (attachment.image) {
                            mu.platform.decodeImage(attachment.data, size)?.let {
                                state.previews[path] = it
                            }
                        }
                        state.insert(mention(path))
                    } catch (e: MuException) {
                        mu.toast(e.message)
                    }
                }
            } finally {
                state.uploading--
            }
        }
    }

    // A plus turned an eighth of the way round is a cross.
    val turn by
        animateFloatAsState(
            if (open) -45f else 0f,
            spring(dampingRatio = 0.6f, stiffness = Spring.StiffnessMedium),
        )
    Box {
        ToolButton(
            Solar.AddCircleBoldDuotone,
            if (open) "Close" else "Attach",
            Modifier.graphicsLayer { rotationZ = turn },
        ) {
            open = !open
        }
        WindowListPopup(
            show = open,
            alignment = PopupPositionProvider.Align.Start,
            onDismissRequest = { open = false },
        ) {
            val dismiss = LocalDismissState.current
            val choose = { action: () -> Unit ->
                dismiss?.invoke()
                action()
            }
            Column(
                Modifier.width(260.dp).background(MiuixTheme.colorScheme.surface).padding(8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    AttachTile(Solar.CameraMinimalisticBoldDuotone, "Camera") {
                        choose { pick { listOfNotNull(mu.platform.takePhoto()) } }
                    }
                    AttachTile(Solar.AlbumBoldDuotone, "Photos") {
                        choose { pick { mu.platform.pickImages() } }
                    }
                    AttachTile(Solar.FileStackBoldDuotone, "Files") {
                        choose { pick { mu.platform.pickFiles() } }
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    AttachTile(Solar.ToolboxBoldDuotone, "Skills") {
                        choose {
                            state.useShell(false)
                            state.field.setTextAndPlaceCursorAtEnd("/")
                            state.useKeyboard(true)
                        }
                    }
                    AttachTile(Solar.FileTerminalBoldDuotone, "Shell") {
                        choose { state.useShell(true) }
                    }
                }
            }
        }
    }
}

@Composable
private fun RowScope.AttachTile(icon: ImageVector, label: String, onClick: () -> Unit) {
    Card(
        modifier = Modifier.weight(1f),
        colors = CardDefaults.defaultColors(color = MiuixTheme.colorScheme.surfaceContainer),
        onClick = onClick,
    ) {
        Box(Modifier.fillMaxWidth().height(56.dp), contentAlignment = Alignment.Center) {
            Icon(icon, label, Modifier.size(30.dp), tint = Accent)
        }
    }
}
