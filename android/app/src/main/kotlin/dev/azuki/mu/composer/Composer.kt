package dev.azuki.mu.composer

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.Mu
import dev.azuki.mu.baseName
import dev.azuki.mu.chat.Destination
import dev.azuki.mu.core.MuException
import dev.azuki.mu.core.mentions
import dev.azuki.mu.design.Glyphs
import dev.azuki.mu.design.Mono
import dev.azuki.mu.design.Pictograms
import dev.azuki.mu.design.SnippetIcon
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.color
import dev.azuki.mu.design.icon
import dev.azuki.mu.design.lifted
import dev.azuki.mu.wire.Mention
import dev.azuki.mu.wire.Resolved
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.IconButton
import top.yukonga.miuix.kmp.basic.InfiniteProgressIndicator
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

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
                val unreachable = mu.destination.let { it.host != null && !it.ready }
                when {
                    state.busy || state.uploading > 0 || unreachable -> Unit
                    state.shell ->
                        if (trimmed.isNotEmpty()) {
                            state.field.clearText()
                            state.snippets = false
                            currentOnSubmit(Submission.Command(trimmed))
                        }

                    trimmed.isEmpty() -> Unit
                    else -> {
                        state.field.clearText()
                        state.previews.clear()
                        currentOnSubmit(Submission.Prompt(trimmed, invoked(trimmed, currentSkills)))
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
                        Suggestion(
                            Pictograms.vector(it.pictogram?.name, SnippetIcon),
                            it.name,
                            it.command,
                            it.pictogram.color,
                        )
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
                        canRun = destination.ready,
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
            tiles(text, selection, mentions(text), resolved)
        }
    Column {
        val host = destination.host
        if (host != null && tiles.isNotEmpty()) AttachmentStrip(state, mu, host, text, tiles)
        Input(
            state.field,
            placeholder = if (running) "Steer…" else "Waiting for a Human Intelligence…",
            mono = false,
            modifier = Modifier.focusRequester(focus),
            outputTransformation = remember(resolved) { coloring(resolved) },
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
                    StopButton(state.stopMenu) { children ->
                        mu.launch { destination.stop(children) }
                    }

                else ->
                    RoundAction(
                        Glyphs.Send,
                        "Send",
                        enabled = !empty && destination.ready,
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
        cursorBrush = SolidColor(MiuixTheme.colorScheme.primary),
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
internal fun ToolButton(
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
        backgroundColor =
            if (enabled) MiuixTheme.colorScheme.primary
            else MiuixTheme.colorScheme.disabledPrimaryButton,
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
                    if (enabled) MiuixTheme.colorScheme.onPrimary
                    else MiuixTheme.colorScheme.surface,
            )
        }
    }
}

private class Suggestion(
    val icon: ImageVector,
    val title: String,
    val detail: String?,
    val tint: Color? = null,
)

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
                    Icon(
                        item.icon,
                        null,
                        Modifier.size(22.dp),
                        tint = item.tint ?: MiuixTheme.colorScheme.primary,
                    )
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
                        mu.remote(host)
                            .completeMention(directory, text, chars(text, selection.start), refresh)
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

private fun chars(text: String, utf16: Int): ULong =
    (0 until utf16).count { !text[it].isLowSurrogate() }.toULong()

fun utf16(text: String, chars: ULong): Int {
    var index = 0
    repeat(chars.toInt()) {
        if (index >= text.length) return text.length
        index += if (text[index].isHighSurrogate() && index + 1 < text.length) 2 else 1
    }
    return index
}

// Floats what it holds above where it sits, taking up no room, so the conversation stays put.
internal fun Modifier.above(gap: Dp) = layout { measurable, constraints ->
    val placeable =
        measurable.measure(constraints.copy(minHeight = 0, maxHeight = Constraints.Infinity))
    layout(placeable.width, 0) { placeable.place(0, -placeable.height - gap.roundToPx()) }
}
