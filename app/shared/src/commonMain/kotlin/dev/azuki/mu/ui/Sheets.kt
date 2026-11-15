package dev.azuki.mu.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.model.Directory
import dev.azuki.mu.model.EntryKind
import dev.azuki.mu.model.MuException
import dev.azuki.mu.model.Snippet
import dev.azuki.mu.state.Mu
import dev.azuki.mu.state.parentOf
import dev.azuki.mu.state.tilde
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.BasicComponent
import top.yukonga.miuix.kmp.basic.ButtonDefaults
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.InfiniteProgressIndicator
import top.yukonga.miuix.kmp.basic.SmallTitle
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.basic.TextButton
import top.yukonga.miuix.kmp.basic.TextField
import top.yukonga.miuix.kmp.preference.ArrowPreference
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
private fun SheetClose() {
    val dismiss = LocalDismissState.current
    CloseButton(onClick = { dismiss?.invoke() })
}

private enum class SheetSlot {
    Header,
    Body,
    Footer,
}

@Composable
private fun Sheet(
    show: Boolean,
    title: String,
    onDismissRequest: () -> Unit,
    onDismissFinished: (() -> Unit)? = null,
    startAction: @Composable () -> Unit = { SheetClose() },
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
            val height =
                maxOf(body.maxOfOrNull { it.height } ?: 0, top + bottom)
                    .coerceIn(constraints.minHeight, constraints.maxHeight)
            layout(constraints.maxWidth, height) {
                body.forEach { it.place(0, 0) }
                header.forEach { it.place(0, 0) }
                footer.forEach { it.place(0, height - it.height) }
            }
        }
    }
}

@Composable
private fun SheetColumn(
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
private fun PrimaryButton(text: String, enabled: Boolean = true, onClick: () -> Unit) {
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
private fun Note(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        modifier.padding(horizontal = 12.dp),
        style = MiuixTheme.textStyles.footnote1,
        color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
    )
}

@Composable
fun AddHostSheet(show: Boolean, mu: Mu, onDismiss: () -> Unit, invite: String? = null) {
    var help by remember { mutableStateOf(false) }
    var link by remember(show, invite) { mutableStateOf(invite.orEmpty()) }
    var pairing by remember { mutableStateOf(false) }
    var scanning by remember(show) { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    fun pair(invite: String) {
        if (pairing) return
        link = invite
        scanning = false
        pairing = true
        scope.launch {
            try {
                val host = mu.pair(invite.trim())
                mu.toast("Paired with ${host.name}")
                onDismiss()
            } catch (e: MuException) {
                mu.toast(e.message)
            } finally {
                pairing = false
            }
        }
    }
    Sheet(
        show = show,
        title = if (scanning) "Scan QR Code" else "Add New Host",
        onDismissRequest = onDismiss,
        startAction = {
            if (scanning) BackButton(onClick = { scanning = false }) else SheetClose()
        },
        endAction = {
            CircleButton(
                Solar.ChatRoundQuestionMarkBoldDuotone,
                "Help",
                { help = true },
                tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            )
        },
        bottomAction =
            if (scanning) null
            else {
                {
                    PrimaryButton(
                        text =
                            when {
                                pairing -> "Pairing…"
                                link.isNotBlank() -> "Pair"
                                else -> "Scan QR"
                            },
                        enabled = !pairing,
                    ) {
                        if (link.isNotBlank()) pair(link)
                        else
                            scope.launch {
                                if (mu.platform.allowCamera()) {
                                    scanning = true
                                } else {
                                    mu.toast(
                                        "Scanning needs the camera. Scan with the camera app, or paste the link"
                                    )
                                }
                            }
                    }
                }
            },
    ) { padding ->
        SheetColumn(padding) {
            if (scanning) {
                mu.platform.QrScanner(
                    onScan = ::pair,
                    onError = {
                        scanning = false
                        mu.toast("$it. Scan with the camera app, or paste the link")
                    },
                    modifier =
                        Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(20.dp)),
                )
                Note("Point the camera at the QR code mu pair shows on the host.")
            } else {
                TextField(
                    value = link,
                    onValueChange = { link = it },
                    label = "mu://…",
                    useLabelAsPlaceholder = true,
                    singleLine = true,
                    textStyle =
                        MiuixTheme.textStyles.main.copy(fontFamily = Mono, fontSize = 15.sp),
                )
                Note(
                    "Run mu pair on the host, then scan the QR code it shows, or paste its link above."
                )
            }
        }
    }
    HelpSheet(help, onDismiss = { help = false })
}

@Composable
private fun HelpSheet(show: Boolean, onDismiss: () -> Unit) {
    Sheet(show = show, title = "Help", onDismissRequest = onDismiss) { padding ->
        SheetColumn(padding, verticalArrangement = Arrangement.Top) {
            SmallTitle(
                "Requirements",
                insideMargin = PaddingValues(start = 12.dp, top = 8.dp, bottom = 8.dp),
            )
            Card {
                BasicComponent(
                    title = "Accessible by this Device",
                    summary =
                        "Hosts must be accessible by this device over the network. This may mean LAN, Tailscale, WireGuard or similar.",
                    startAction = { HelpIcon(Solar.GlobalLineDuotone) },
                )
                BasicComponent(
                    title = "mu serve",
                    summary =
                        "To advertise as a host. Set MU_LISTEN, like MU_LISTEN=0.0.0.0:7337, to let devices in over the network.",
                    startAction = { HelpIcon(Solar.DatabaseBoldDuotone) },
                )
                BasicComponent(
                    title = "mu pair",
                    summary =
                        "Shows the invite for one device as a QR code and a link. It expires in 10 minutes.",
                    startAction = { HelpIcon(Solar.FileTerminalBoldDuotone) },
                )
            }
        }
    }
}

@Composable
private fun HelpIcon(icon: ImageVector) {
    Icon(icon, null, Modifier.size(24.dp), tint = Accent)
}

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
            directory = mu.core.listDirectory(host, path)
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
    Icon(icon, null, Modifier.size(24.dp), tint = Accent)
}

@Composable
fun IconPicker(
    show: Boolean,
    current: String?,
    snippet: Boolean,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    Sheet(show = show, title = "Set Icon", onDismissRequest = onDismiss) { padding ->
        val dismiss = LocalDismissState.current
        SheetColumn(padding) {
            Text(
                "Solar Icons by 480 Design",
                Modifier.fillMaxWidth().padding(bottom = 4.dp),
                style = MiuixTheme.textStyles.footnote1,
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                textAlign = TextAlign.Center,
            )
            Pictograms.all.chunked(4).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    row.forEach { pictogram ->
                        val selected = pictogram.name == current
                        Box(
                            Modifier.weight(1f)
                                .aspectRatio(1f)
                                .clip(CircleShape)
                                .background(
                                    if (selected) MiuixTheme.colorScheme.tertiaryContainer
                                    else MiuixTheme.colorScheme.surfaceContainer
                                )
                                .clickable {
                                    onPick(pictogram.name)
                                    dismiss?.invoke()
                                },
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(
                                pictogram.vector,
                                null,
                                Modifier.size(36.dp),
                                tint = if (snippet) Accent else pictogram.tint,
                            )
                        }
                    }
                }
            }
        }
    }
}

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
                                mu.core.deleteSnippet(host, snippet.id)
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
                        mu.core.saveSnippet(host, snippet?.id, name.trim(), command.trim(), icon)
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
