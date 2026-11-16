package dev.azuki.mu.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import dev.azuki.mu.Mu
import dev.azuki.mu.api.Directory
import dev.azuki.mu.api.EntryKind
import dev.azuki.mu.api.MuException
import dev.azuki.mu.design.BackButton
import dev.azuki.mu.design.Note
import dev.azuki.mu.design.PrimaryButton
import dev.azuki.mu.design.Sheet
import dev.azuki.mu.design.SheetClose
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.icon
import dev.azuki.mu.parentOf
import dev.azuki.mu.tilde
import top.yukonga.miuix.kmp.basic.BasicComponent
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.InfiniteProgressIndicator
import top.yukonga.miuix.kmp.preference.ArrowPreference
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.utils.overScrollVertical

@Composable
fun ProjectPicker(
    show: Boolean,
    mu: Mu,
    host: String,
    start: String?,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
    onDismissFinished: () -> Unit,
) {
    val history = remember(show) { mutableStateListOf(start ?: "~") }
    var directory by remember { mutableStateOf<Directory?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val path = history.last()
    LaunchedEffect(show, host, path) {
        if (!show) return@LaunchedEffect
        directory = null
        error = null
        try {
            directory = mu.remote(host).listDirectory(path)
        } catch (e: MuException) {
            error = e.message
        }
    }
    val shown = directory?.path ?: path
    Sheet(
        show = show,
        title = tilde(shown, mu.home(host)),
        onDismissRequest = onDismiss,
        onDismissFinished = onDismissFinished,
        startAction = {
            if (history.size > 1) BackButton(onClick = { history.removeAt(history.lastIndex) })
            else SheetClose()
        },
        bottomAction = {
            val dismiss = LocalDismissState.current
            PrimaryButton("Use Current Directory", enabled = directory != null) {
                directory?.let { onPick(it.path) }
                dismiss?.invoke()
            }
        },
    ) { padding ->
        LazyColumn(
            Modifier.overScrollVertical(),
            state = remember(path) { LazyListState() },
            contentPadding = padding,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (shown != "/" && shown != "~") {
                item {
                    Card {
                        ArrowPreference(
                            title = "..",
                            startAction = { EntryIcon(Solar.MultipleForwardLeftBoldDuotone) },
                            onClick = { history.add(parentOf(shown)) },
                        )
                    }
                }
            }
            item {
                Card(Modifier.fillMaxWidth()) {
                    val listed = directory
                    when {
                        error != null -> Note(error.orEmpty(), Modifier.padding(16.dp))
                        listed == null ->
                            Box(Modifier.fillMaxWidth().padding(24.dp), Alignment.Center) {
                                InfiniteProgressIndicator()
                            }

                        listed.entries.isEmpty() -> Note("Nothing in here", Modifier.padding(16.dp))
                        else ->
                            listed.entries.forEach { entry ->
                                when (entry.kind) {
                                    EntryKind.File ->
                                        BasicComponent(
                                            title = entry.name,
                                            startAction = { EntryIcon(Solar.FileTextBoldDuotone) },
                                        )

                                    else ->
                                        ArrowPreference(
                                            title = entry.name,
                                            startAction = {
                                                EntryIcon(
                                                    if (entry.kind == EntryKind.Repository)
                                                        Solar.FolderGitBoldDuotone
                                                    else Solar.Folder2BoldDuotone
                                                )
                                            },
                                            onClick = {
                                                history.add(
                                                    "${listed.path.trimEnd('/')}/${entry.name}"
                                                )
                                            },
                                        )
                                }
                            }
                    }
                }
            }
        }
    }
}

@Composable
private fun EntryIcon(icon: ImageVector) {
    Icon(icon, null, Modifier.size(24.dp), tint = MiuixTheme.colorScheme.primary)
}
