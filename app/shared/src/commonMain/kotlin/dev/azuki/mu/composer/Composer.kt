package dev.azuki.mu.composer

import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.OutputTransformation
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.Velocity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.Mu
import dev.azuki.mu.api.Mention
import dev.azuki.mu.api.MuException
import dev.azuki.mu.api.Resolved
import dev.azuki.mu.api.ToolStatus
import dev.azuki.mu.baseName
import dev.azuki.mu.chat.Destination
import dev.azuki.mu.compact
import dev.azuki.mu.design.Accent
import dev.azuki.mu.design.DialogButtons
import dev.azuki.mu.design.Glyphs
import dev.azuki.mu.design.Mono
import dev.azuki.mu.design.Pictograms
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.icon
import dev.azuki.mu.design.lifted
import dev.azuki.mu.platform.Attachment
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.CardDefaults
import top.yukonga.miuix.kmp.basic.DropdownEntry
import top.yukonga.miuix.kmp.basic.DropdownItem
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.IconButton
import top.yukonga.miuix.kmp.basic.InfiniteProgressIndicator
import top.yukonga.miuix.kmp.basic.PopupPositionProvider
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.basic.TextField
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.window.WindowCascadingListPopup
import top.yukonga.miuix.kmp.window.WindowDialog
import top.yukonga.miuix.kmp.window.WindowListPopup

val Efforts = listOf("minimal", "low", "medium", "high", "xhigh")

@Composable
fun Composer(
    state: ComposerState,
    mu: Mu,
    destination: Destination,
    onSubmit: (Submission) -> Unit,
    modifier: Modifier = Modifier,
) {
    val skills = destination.skills
    val snippets = destination.host?.let { mu.live[it]?.profile?.snippets }.orEmpty()
    val text = state.field.text.toString()
    val slash =
        text.takeIf { !state.shell && it.startsWith("/") && it.none(Char::isWhitespace) }?.drop(1)
    val suggestions =
        slash
            ?.let { query -> skills.filter { it.name.startsWith(query, ignoreCase = true) } }
            .orEmpty()

    val mention = rememberMention(state, mu, destination)
    val resolved = rememberResolved(state, mu, destination)

    val currentSkills by rememberUpdatedState(skills)
    val currentOnSubmit by rememberUpdatedState(onSubmit)
    // Reads everything as it is at the tap, not as it was composed.
    val submit =
        remember(state) {
            {
                val trimmed = state.field.text.toString().trim()
                when {
                    state.busy || state.uploading > 0 -> Unit
                    state.shell ->
                        if (trimmed.isNotEmpty()) {
                            state.field.clearText()
                            state.snippets = false
                            currentOnSubmit(Submission.Command(trimmed))
                        }

                    trimmed.isEmpty() -> Unit
                    else -> {
                        val word =
                            trimmed.takeIf { it.startsWith("/") }?.substringBefore(' ')?.drop(1)
                        val skill = currentSkills.firstOrNull { it.name == word }?.name
                        state.field.clearText()
                        state.previews.clear()
                        currentOnSubmit(Submission.Prompt(trimmed, skill))
                    }
                }
            }
        }

    Column(modifier.padding(horizontal = 12.dp)) {
        state.run?.let { run -> key(run) { ShellOutput(run, onDismiss = { state.run = null }) } }
        Column(
            Modifier.fillMaxWidth().above(8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (state.shell && state.snippets && snippets.isNotEmpty()) {
                Suggestions(
                    snippets.map {
                        Suggestion(Pictograms.snippet(it.icon).vector, it.name, it.command)
                    }
                ) { index ->
                    state.field.setTextAndPlaceCursorAtEnd(snippets[index].command)
                    state.snippets = false
                }
            }
            val mentioned = mention?.takeIf { suggestions.isEmpty() && !state.shell }
            val files = mentioned?.mention?.entries.orEmpty()
            if (files.isNotEmpty()) {
                Suggestions(
                    files.map { entry ->
                        val directory = entry.label.endsWith('/')
                        val name = baseName(entry.label) + if (directory) "/" else ""
                        Suggestion(
                            if (directory) Solar.Folder2BoldDuotone else Solar.FileTextBoldDuotone,
                            name,
                            entry.label.removeSuffix("/").substringBeforeLast('/', "").takeIf {
                                it.isNotEmpty()
                            },
                        )
                    }
                ) { index ->
                    val found = mentioned ?: return@Suggestions
                    if (state.field.text.toString() != found.text) return@Suggestions
                    val replacement = files[index].text
                    val start = utf16(found.text, found.mention.start)
                    val end = utf16(found.text, found.mention.end)
                    state.field.edit {
                        replace(start, end, replacement)
                        selection = TextRange(start + replacement.length)
                    }
                }
            }
            if (suggestions.isNotEmpty()) {
                Suggestions(
                    suggestions.map {
                        Suggestion(Solar.ToolboxBoldDuotone, it.name, "${it.description} (Skill)")
                    }
                ) { index ->
                    state.field.setTextAndPlaceCursorAtEnd("/${suggestions[index].name} ")
                }
            }
        }
        Card(
            cornerRadius = 24.dp,
            modifier =
                Modifier.lifted(RoundedCornerShape(24.dp))
                    .animateContentSize(alignment = Alignment.BottomCenter),
        ) {
            when {
                state.shell ->
                    ShellInput(
                        state,
                        canRun = destination.host != null,
                        hasSnippets = snippets.isNotEmpty(),
                        onSubmit = submit,
                    )

                !state.keyboard && text.isEmpty() && state.uploading == 0 ->
                    DictationBar(state, mu, destination, onSubmit)

                else -> PromptInput(state, mu, destination, resolved, onSubmit = submit)
            }
        }
    }
}

@Composable
private fun rememberRequestedFocus(state: ComposerState): FocusRequester {
    val focus = remember { FocusRequester() }
    LaunchedEffect(state.focusRequested) {
        if (state.focusRequested) {
            runCatching { focus.requestFocus() }
            state.focusRequested = false
        }
    }
    return focus
}

@Composable
private fun PromptInput(
    state: ComposerState,
    mu: Mu,
    destination: Destination,
    resolved: Map<String, Resolved>,
    onSubmit: () -> Unit,
) {
    val running = destination.running
    val focus = rememberRequestedFocus(state)
    val text = state.field.text.toString()
    val empty = text.isBlank() && state.uploading == 0
    val selection = state.field.selection
    val tiles =
        remember(text, selection, resolved) {
            tiles(text, selection, mu.core.mentions(text), resolved)
        }
    Column {
        val host = destination.host
        if (host != null && tiles.isNotEmpty()) AttachmentStrip(state, mu, host, text, tiles)
        Input(
            state.field,
            placeholder = if (running) "Steer…" else "Waiting for a Human Intelligence…",
            mono = false,
            modifier = Modifier.focusRequester(focus),
            outputTransformation = remember(resolved) { coloring(mu.core, resolved) },
        )
        Row(
            Modifier.padding(horizontal = 8.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            AttachButton(state, mu, destination)
            ModelButton(mu, destination)
            Spacer(Modifier.weight(1f))
            ToolButton(
                Solar.WiFiHighLineDuotone,
                "Dictate",
                Modifier.graphicsLayer { rotationZ = 90f },
            ) {
                state.useKeyboard(false)
            }
            Spacer(Modifier.width(4.dp))
            when {
                running && empty ->
                    RoundAction(Glyphs.Stop, "Stop", enabled = true) {
                        mu.launch { destination.stop() }
                    }

                else ->
                    RoundAction(
                        Glyphs.Send,
                        "Send",
                        enabled = !empty && destination.host != null,
                        busy = state.busy || state.uploading > 0,
                        onClick = onSubmit,
                    )
            }
        }
    }
}

@Composable
private fun ShellInput(
    state: ComposerState,
    canRun: Boolean,
    hasSnippets: Boolean,
    onSubmit: () -> Unit,
) {
    val focus = rememberRequestedFocus(state)
    Column {
        Input(
            state.field,
            "Time to hack the mainframe? (doge)",
            mono = true,
            modifier = Modifier.focusRequester(focus),
        )
        Row(
            Modifier.padding(horizontal = 8.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ToolButton(Solar.RoundAltArrowLeftBoldDuotone, "Leave shell mode") {
                state.useShell(false)
            }
            val showingSnippets = state.snippets && hasSnippets
            ToolButton(
                if (showingSnippets) Solar.CloseCircleBoldDuotone
                else Solar.BookmarkCircleBoldDuotone,
                if (showingSnippets) "Hide snippets" else "Snippets",
                enabled = hasSnippets,
            ) {
                state.snippets = !state.snippets
            }
            Spacer(Modifier.weight(1f))
            RoundAction(
                Glyphs.Return,
                "Run",
                enabled = canRun && state.field.text.isNotBlank(),
                onClick = onSubmit,
            )
        }
    }
}

@Composable
private fun DictationBar(
    state: ComposerState,
    mu: Mu,
    destination: Destination,
    onSubmit: (Submission) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val dictation = mu.platform.dictation
    Row(
        Modifier.padding(horizontal = 8.dp).height(56.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        AttachButton(state, mu, destination)
        ModelButton(mu, destination)
        Box(
            Modifier.weight(1f).height(56.dp).pointerInput(Unit) {
                awaitEachGesture {
                    val down = awaitFirstDown()
                    down.consume()
                    haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                    state.dictating = true
                    state.cancelArmed = false
                    var listening = false
                    val start = scope.launch {
                        listening =
                            try {
                                dictation.start()
                            } catch (e: MuException) {
                                mu.toast(e.message)
                                false
                            }
                        if (!listening) state.dictating = false
                    }
                    val threshold = 72.dp.toPx()
                    while (true) {
                        val change =
                            awaitPointerEvent().changes.firstOrNull { it.id == down.id } ?: break
                        state.cancelArmed = down.position.y - change.position.y > threshold
                        change.consume()
                        if (!change.pressed) break
                    }
                    val cancel = state.cancelArmed
                    state.dictating = false
                    state.cancelArmed = false
                    scope.launch {
                        start.join()
                        if (cancel || !listening) {
                            dictation.cancel()
                            return@launch
                        }
                        val text = dictation.finish().trim()
                        if (text.isNotEmpty()) onSubmit(Submission.Prompt(text, null))
                    }
                }
            },
            contentAlignment = Alignment.Center,
        ) {
            Text(
                "Hold to Dictate",
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                style = MiuixTheme.textStyles.body1,
            )
        }
        ToolButton(Solar.KeyboardBoldDuotone, "Type") { state.useKeyboard(true) }
    }
}

@Composable
private fun Input(
    field: TextFieldState,
    placeholder: String,
    mono: Boolean,
    modifier: Modifier = Modifier,
    outputTransformation: OutputTransformation? = null,
) {
    val style =
        MiuixTheme.textStyles.body1.let {
            if (mono) it.copy(fontFamily = Mono, fontSize = 15.sp) else it
        }
    BasicTextField(
        state = field,
        modifier = modifier.fillMaxWidth().padding(start = 18.dp, end = 18.dp, top = 16.dp),
        keyboardOptions =
            if (mono) KeyboardOptions(autoCorrectEnabled = false) else KeyboardOptions.Default,
        textStyle = style.copy(color = MiuixTheme.colorScheme.onSurface),
        lineLimits = TextFieldLineLimits.MultiLine(1, 6),
        cursorBrush = SolidColor(Accent),
        outputTransformation = outputTransformation,
        decorator = { inner ->
            Box {
                if (field.text.isEmpty()) {
                    Text(
                        placeholder,
                        style = style,
                        color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                        maxLines = 1,
                    )
                }
                inner()
            }
        },
    )
}

@Composable
private fun ToolButton(
    icon: ImageVector,
    contentDescription: String,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    onClick: () -> Unit,
) {
    IconButton(onClick = onClick, enabled = enabled, minWidth = 36.dp, minHeight = 36.dp) {
        Icon(
            icon,
            contentDescription,
            modifier.size(24.dp),
            tint =
                MiuixTheme.colorScheme.onSurfaceVariantSummary.let {
                    if (enabled) it else it.copy(alpha = 0.4f)
                },
        )
    }
}

@Composable
private fun RoundAction(
    icon: ImageVector,
    contentDescription: String,
    enabled: Boolean,
    busy: Boolean = false,
    onClick: () -> Unit,
) {
    IconButton(
        onClick = onClick,
        enabled = enabled && !busy,
        backgroundColor = if (enabled) Accent else MiuixTheme.colorScheme.disabledPrimaryButton,
        minWidth = 36.dp,
        minHeight = 36.dp,
    ) {
        if (busy) {
            InfiniteProgressIndicator(color = MiuixTheme.colorScheme.onPrimary, size = 18.dp)
        } else {
            Icon(
                icon,
                contentDescription,
                Modifier.size(20.dp),
                tint =
                    MiuixTheme.colorScheme.onPrimary.let {
                        if (enabled) it else it.copy(alpha = 0.5f)
                    },
            )
        }
    }
}

@Composable
private fun AttachButton(state: ComposerState, mu: Mu, destination: Destination) {
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
                        val path = mu.core.upload(host, attachment.name, attachment.data)
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

@Composable
private fun ModelButton(mu: Mu, destination: Destination) {
    var custom by remember { mutableStateOf(false) }
    val model = destination.model
    val effort = destination.effort
    val pick = { picked: String -> destination.use(picked, effort) }
    val models = destination.host?.let { mu.models[it] }.orEmpty()
    val modelItems = buildList {
        if (model != null && models.none { it.name == model }) {
            add(DropdownItem(model, selected = true, onClick = {}))
        }
        models.forEach {
            add(
                DropdownItem(
                    it.name,
                    summary = it.context?.let(::compact),
                    selected = it.name == model,
                    onClick = { pick(it.name) },
                )
            )
        }
        add(DropdownItem("Other…", onClick = { custom = true }))
    }
    val effortItems =
        listOf(
            DropdownItem(
                "Default",
                selected = effort == null,
                onClick = { destination.use(model, null) },
            )
        ) +
            Efforts.map {
                DropdownItem(it, selected = it == effort, onClick = { destination.use(model, it) })
            }
    val entry =
        DropdownEntry(
            listOf(
                DropdownItem(
                    "Model",
                    summary = model ?: "None",
                    children = modelItems,
                ),
                DropdownItem("Effort", summary = effort ?: "Default", children = effortItems),
            )
        )
    var open by remember { mutableStateOf(false) }
    var held by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    Box {
        IconButton(
            onClick = {
                open = !open
                if (open) {
                    held = true
                    haptics.performHapticFeedback(HapticFeedbackType.ContextClick)
                }
            },
            holdDownState = held,
            minWidth = 36.dp,
            minHeight = 36.dp,
        ) {
            Icon(
                Solar.SpeedometerMaxBoldDuotone,
                "Model and effort",
                Modifier.size(24.dp),
                tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            )
        }
        WindowCascadingListPopup(
            show = open,
            entries = listOf(entry),
            alignment = PopupPositionProvider.Align.Start,
            onDismissRequest = { open = false },
            onDismissFinished = { held = false },
        )
    }
    ModelDialog(custom, onDismiss = { custom = false }, onPick = pick)
}

@Composable
private fun ModelDialog(show: Boolean, onDismiss: () -> Unit, onPick: (String) -> Unit) {
    var name by remember(show) { mutableStateOf("") }
    WindowDialog(
        show = show,
        title = "Model",
        summary = "Any model the host's endpoint serves",
        onDismissRequest = onDismiss,
    ) {
        val dismiss = LocalDismissState.current
        TextField(
            value = name,
            onValueChange = { name = it },
            label = "Model",
            useLabelAsPlaceholder = true,
            singleLine = true,
            textStyle = MiuixTheme.textStyles.main.copy(fontFamily = Mono),
            modifier = Modifier.padding(bottom = 16.dp),
        )
        DialogButtons(
            confirm = "Use",
            enabled = name.isNotBlank(),
            onCancel = { dismiss?.invoke() },
            onConfirm = {
                onPick(name.trim())
                dismiss?.invoke()
            },
        )
    }
}

private class Suggestion(val icon: ImageVector, val title: String, val detail: String?)

@Composable
private fun Suggestions(items: List<Suggestion>, onPick: (Int) -> Unit) {
    Card(cornerRadius = 20.dp, modifier = Modifier.lifted(RoundedCornerShape(20.dp))) {
        Column(
            Modifier.heightIn(max = 236.dp)
                .verticalScroll(rememberScrollState())
                .padding(vertical = 6.dp)
        ) {
            items.forEachIndexed { index, item ->
                Row(
                    Modifier.fillMaxWidth()
                        .clickable { onPick(index) }
                        .padding(horizontal = 16.dp, vertical = 11.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(item.icon, null, Modifier.size(22.dp), tint = Accent)
                    Text(
                        item.title,
                        Modifier.padding(start = 10.dp),
                        style = MiuixTheme.textStyles.body1,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    item.detail?.let { detail ->
                        Text(
                            detail,
                            Modifier.padding(start = 8.dp).weight(1f, fill = false),
                            fontFamily = Mono,
                            fontSize = 12.sp,
                            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
        }
    }
}

private class Mentioned(val text: String, val mention: Mention)

// The host completes mentions, so each edit asks it again; a newer edit drops the older ask.
@Composable
private fun rememberMention(state: ComposerState, mu: Mu, destination: Destination): Mentioned? {
    val host = destination.host
    val directory = destination.directory
    var found by remember(host, directory) { mutableStateOf<Mentioned?>(null) }
    LaunchedEffect(state, host, directory) {
        var open = false
        snapshotFlow { Triple(state.shell, state.field.text.toString(), state.field.selection) }
            .collectLatest { (shell, text, selection) ->
                if (
                    host == null ||
                        directory == null ||
                        shell ||
                        !selection.collapsed ||
                        '@' !in text
                ) {
                    found = null
                    open = false
                    return@collectLatest
                }
                val refresh = !open
                open = true
                val mention =
                    try {
                        mu.core.completeMention(
                            host,
                            directory,
                            text,
                            chars(text, selection.start),
                            refresh,
                        )
                    } catch (e: MuException) {
                        if (refresh) mu.toast("File search: ${e.message}")
                        null
                    }
                if (mention == null) open = false
                found = mention?.let { Mentioned(text, it) }
            }
    }
    return found
}

private fun chars(text: String, utf16: Int): Int =
    (0 until utf16).count { !text[it].isLowSurrogate() }

fun utf16(text: String, chars: Int): Int {
    var index = 0
    repeat(chars) {
        if (index >= text.length) return text.length
        index += if (text[index].isHighSurrogate() && index + 1 < text.length) 2 else 1
    }
    return index
}

// Floats what it holds above where it sits, taking up no room, so the conversation stays put.
private fun Modifier.above(gap: Dp) = layout { measurable, constraints ->
    val placeable =
        measurable.measure(constraints.copy(minHeight = 0, maxHeight = Constraints.Infinity))
    layout(placeable.width, 0) { placeable.place(0, -placeable.height - gap.roundToPx()) }
}

// The output of a command run from the composer, peeking out from behind it. Swiping it down
// tucks it away to just its handle; swiping once more from there stops the command and drops it.
@Composable
private fun ShellOutput(run: ShellRun, onDismiss: () -> Unit) {
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
                val status =
                    when (val status = run.status) {
                        is ToolStatus.Exited ->
                            if (status.code == 0) null else "exit ${status.code}"

                        is ToolStatus.TimedOut -> "timed out"
                        ToolStatus.Cancelled -> "cancelled"
                        ToolStatus.Failed -> "failed"
                        else -> null
                    }
                Text(
                    buildString {
                        append("$ ").append(run.command)
                        if (run.output.isNotEmpty()) append('\n').append(run.output.trimEnd())
                        run.failure?.let { append('\n').append(it) }
                        status?.let { append("\n[").append(it).append(']') }
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
