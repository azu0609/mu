package dev.azuki.mu.settings

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import dev.azuki.mu.Mu
import dev.azuki.mu.design.DialogButtons
import dev.azuki.mu.design.Presented
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.window.WindowDialog

@Composable
fun DeleteSessionsDialog(mu: Mu, presented: Presented<String>) {
    val host = presented.value
    val count = host?.let { mu.live[it]?.sessions?.size } ?: 0
    WindowDialog(
        show = presented.show,
        title = "Are you sure?",
        summary =
            host?.let {
                "${if (count == 1) "1 session" else "$count sessions"} on “${mu.name(it)}” will be deleted permanently."
            },
        onDismissRequest = presented::dismiss,
        onDismissFinished = presented::finished,
    ) {
        val dismiss = LocalDismissState.current
        DialogButtons(
            confirm = "Delete",
            destructive = true,
            onCancel = { dismiss?.invoke() },
            onConfirm = {
                host?.let {
                    mu.launch {
                        val deleted = mu.remote(it).deleteSessions()
                        mu.toast(
                            buildString {
                                append(
                                    "Deleted ${deleted.deleted} ${if (deleted.deleted == 1L) "session" else "sessions"}"
                                )
                                if (deleted.inUse > 0) append("; ${deleted.inUse} in use were kept")
                            }
                        )
                    }
                }
                dismiss?.invoke()
            },
        )
    }
}
