package dev.azuki.mu.settings

import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.Mu
import dev.azuki.mu.api.MuException
import dev.azuki.mu.api.Snippet
import dev.azuki.mu.design.CircleButton
import dev.azuki.mu.design.Danger
import dev.azuki.mu.design.Mono
import dev.azuki.mu.design.Note
import dev.azuki.mu.design.PictogramBadge
import dev.azuki.mu.design.Pictograms
import dev.azuki.mu.design.PrimaryButton
import dev.azuki.mu.design.Sheet
import dev.azuki.mu.design.SheetColumn
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.icon
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.TextField
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme

@Composable
fun SnippetSheet(
    show: Boolean,
    mu: Mu,
    host: String,
    snippet: Snippet?,
    onDismiss: () -> Unit,
    onDismissFinished: () -> Unit,
) {
    var name by remember(snippet, show) { mutableStateOf(snippet?.name.orEmpty()) }
    var command by remember(snippet, show) { mutableStateOf(snippet?.command.orEmpty()) }
    var icon by
        remember(snippet, show) {
            mutableStateOf(snippet?.icon ?: "solar:box-minimalistic-bold-duotone")
        }
    var picking by remember { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    Sheet(
        show = show,
        title = if (snippet == null) "New Bash Snippet" else "Edit Bash Snippet",
        onDismissRequest = onDismiss,
        onDismissFinished = onDismissFinished,
        endAction = {
            if (snippet != null) {
                val dismiss = LocalDismissState.current
                CircleButton(
                    Solar.TrashBinTrashBoldDuotone,
                    "Delete",
                    tint = Danger,
                    onClick = {
                        scope.launch {
                            try {
                                mu.remote(host).deleteSnippet(snippet.id)
                                dismiss?.invoke()
                            } catch (e: MuException) {
                                mu.toast(e.message)
                            }
                        }
                    },
                )
            }
        },
        bottomAction = {
            val dismiss = LocalDismissState.current
            PrimaryButton(
                if (snippet == null) "Add" else "Save",
                enabled = name.isNotBlank() && command.isNotBlank() && !saving,
            ) {
                saving = true
                scope.launch {
                    try {
                        mu.remote(host).saveSnippet(snippet?.id, name.trim(), command.trim(), icon)
                        dismiss?.invoke()
                    } catch (e: MuException) {
                        mu.toast(e.message)
                    } finally {
                        saving = false
                    }
                }
            }
        },
    ) { padding ->
        SheetColumn(padding) {
            PictogramBadge(
                Pictograms.snippet(icon),
                Modifier.align(Alignment.CenterHorizontally).padding(vertical = 8.dp),
                size = 88.dp,
                onClick = { picking = true },
            )
            TextField(
                value = name,
                onValueChange = { name = it },
                label = "Name",
                useLabelAsPlaceholder = true,
                singleLine = true,
            )
            TextField(
                value = command,
                onValueChange = { command = it },
                label = "Command",
                useLabelAsPlaceholder = true,
                maxLines = 5,
                textStyle = MiuixTheme.textStyles.main.copy(fontFamily = Mono, fontSize = 15.sp),
            )
            Note(
                "Commands invoked this way will inherit a “MU_USER_TRIGGERED” environment set to 1, which may change some of the built-in command behavior compared to when an agent invokes it."
            )
        }
    }
    IconPicker(
        picking,
        icon,
        snippet = true,
        onPick = { icon = it },
        onDismiss = { picking = false },
    )
}
