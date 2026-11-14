package dev.azuki.mu.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.model.Attachment
import dev.azuki.mu.model.MuException
import dev.azuki.mu.model.Prefs
import dev.azuki.mu.model.ToolStatus
import dev.azuki.mu.state.Destination
import dev.azuki.mu.state.Mu
import dev.azuki.mu.state.compact
import kotlinx.coroutines.Job
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

class ComposerState(private val prefs: Prefs) {
    val field = TextFieldState()
    var keyboard by mutableStateOf(prefs[KEYBOARD] == "1")
        private set

    var shell by mutableStateOf(false)
    var snippets by mutableStateOf(false)
    val attachments = mutableStateListOf<Attachment>()
    var busy by mutableStateOf(false)
    var run by mutableStateOf<ShellRun?>(null)
    var dictating by mutableStateOf(false)
    var cancelArmed by mutableStateOf(false)

    fun useKeyboard(on: Boolean) {
        keyboard = on
        prefs[KEYBOARD] = if (on) "1" else null
    }

    private companion object {
        const val KEYBOARD = "composer.keyboard"
    }
}

class ShellRun(val command: String) {
    var output by mutableStateOf("")
    var status by mutableStateOf<ToolStatus>(ToolStatus.Running)
    var failure by mutableStateOf<String?>(null)
    var job: Job? = null
    val done
        get() = status != ToolStatus.Running || failure != null
}

sealed interface Submission {
    data class Prompt(val text: String, val skill: String?, val attachments: List<Attachment>) :
        Submission

    data class Command(val command: String) : Submission
}

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

    val currentSkills by rememberUpdatedState(skills)
    val currentOnSubmit by rememberUpdatedState(onSubmit)
    // Reads everything as it is at the tap, not as it was composed.
    val submit =
        remember(state) {
            {
                val trimmed = state.field.text.toString().trim()
                when {
                    state.busy -> Unit
                    state.shell ->
                        if (trimmed.isNotEmpty()) {
                            state.field.clearText()
                            state.snippets = false
                            currentOnSubmit(Submission.Command(trimmed))
                        }

                    trimmed.isEmpty() && state.attachments.isEmpty() -> Unit
                    else -> {
                        val word =
                            trimmed.takeIf { it.startsWith("/") }?.substringBefore(' ')?.drop(1)
                        val skill = currentSkills.firstOrNull { it.name == word }?.name
                        val attachments = state.attachments.toList()
                        state.field.clearText()
                        state.attachments.clear()
                        currentOnSubmit(Submission.Prompt(trimmed, skill, attachments))
                    }
                }
            }
        }

    Column(modifier.padding(horizontal = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        state.run?.let { run -> ShellOutput(run, onDismiss = { state.run = null }) }
        AnimatedVisibility(
            state.shell && state.snippets && snippets.isNotEmpty(),
            enter = fadeIn() + expandVertically(clip = false),
            exit = fadeOut() + shrinkVertically(clip = false),
        ) {
            Suggestions(
                snippets.map {
                    Suggestion(Pictograms.snippet(it.icon).vector, it.name, it.command)
                }
            ) { index ->
                state.field.setTextAndPlaceCursorAtEnd(snippets[index].command)
                state.snippets = false
            }
        }
        AnimatedVisibility(
            suggestions.isNotEmpty(),
            enter = fadeIn() + expandVertically(clip = false),
            exit = fadeOut() + shrinkVertically(clip = false),
        ) {
            Suggestions(
                suggestions.map {
                    Suggestion(Solar.ToolboxBoldDuotone, it.name, "${it.description} (Skill)")
                }
            ) { index ->
                state.field.setTextAndPlaceCursorAtEnd("/${suggestions[index].name} ")
            }
        }
        Card(
            cornerRadius = 24.dp,
            modifier = Modifier.lifted(RoundedCornerShape(24.dp)).animateContentSize(),
        ) {
            when {
                state.shell ->
                    ShellInput(
                        state,
                        canRun = destination.host != null,
                        hasSnippets = snippets.isNotEmpty(),
                        onSubmit = submit,
                    )

                !state.keyboard && text.isEmpty() && state.attachments.isEmpty() ->
                    DictationBar(state, mu, destination, onSubmit)

                else -> PromptInput(state, mu, destination, onSubmit = submit)
            }
        }
    }
}

@Composable
private fun PromptInput(
    state: ComposerState,
    mu: Mu,
    destination: Destination,
    onSubmit: () -> Unit,
) {
    val running = destination.running
    val focus = remember { FocusRequester() }
    LaunchedEffect(state.keyboard) { if (state.keyboard) runCatching { focus.requestFocus() } }
    val empty = state.field.text.isBlank() && state.attachments.isEmpty()
    Column {
        if (state.attachments.isNotEmpty()) {
            Row(
                Modifier.horizontalScroll(rememberScrollState())
                    .padding(start = 12.dp, end = 12.dp, top = 12.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                state.attachments.forEach { attachment ->
                    AttachmentChip(attachment) { state.attachments.remove(attachment) }
                }
            }
        }
        Input(
            state.field,
            placeholder =
                if (running) "Queue a message for when it's done…"
                else "Waiting for a Human Intelligence…",
            mono = false,
            modifier = Modifier.focusRequester(focus),
        )
        Row(
            Modifier.padding(horizontal = 8.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            AttachButton(state, mu)
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
                        busy = state.busy,
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
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
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
                state.shell = false
                state.snippets = false
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
        AttachButton(state, mu)
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
                        if (text.isNotEmpty()) onSubmit(Submission.Prompt(text, null, emptyList()))
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
private fun AttachButton(state: ComposerState, mu: Mu) {
    var open by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun pick(source: suspend () -> List<Attachment>) {
        scope.launch {
            try {
                val picked = source()
                state.attachments += picked
                if (picked.isNotEmpty()) state.useKeyboard(true)
            } catch (e: MuException) {
                mu.toast(e.message)
            }
        }
    }

    Box {
        ToolButton(
            if (open) Solar.CloseCircleBoldDuotone else Solar.AddCircleBoldDuotone,
            "Attach",
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
                            state.shell = false
                            state.field.setTextAndPlaceCursorAtEnd("/")
                            state.useKeyboard(true)
                        }
                    }
                    AttachTile(Solar.FileTerminalBoldDuotone, "Shell") {
                        choose { state.shell = true }
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
private fun AttachmentChip(attachment: Attachment, onRemove: () -> Unit) {
    Row(
        Modifier.clip(RoundedCornerShape(12.dp))
            .background(MiuixTheme.colorScheme.surface)
            .clickable(onClick = onRemove)
            .padding(start = 10.dp, end = 8.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (attachment.image) Solar.AlbumBoldDuotone else Solar.DocumentBoldDuotone,
            null,
            Modifier.size(18.dp),
            tint = Accent,
        )
        Text(
            attachment.name,
            Modifier.padding(horizontal = 6.dp).widthIn(max = 120.dp),
            style = MiuixTheme.textStyles.footnote1,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Icon(
            Solar.CloseLinear,
            "Remove",
            Modifier.size(14.dp),
            tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
        )
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

private class Suggestion(val icon: ImageVector, val title: String, val detail: String)

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
                    )
                    Text(
                        item.detail,
                        Modifier.padding(start = 8.dp),
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

// The output of a command run from the composer, peeking out from behind it.
@Composable
private fun ShellOutput(run: ShellRun, onDismiss: () -> Unit) {
    val overlap = 24.dp
    val scroll = rememberScrollState()
    LaunchedEffect(run.output) { scroll.scrollTo(scroll.maxValue) }
    Card(
        modifier =
            Modifier.padding(horizontal = 16.dp).layout { measurable, constraints ->
                val placeable = measurable.measure(constraints)
                val hidden = overlap.roundToPx()
                layout(placeable.width, (placeable.height - hidden).coerceAtLeast(0)) {
                    placeable.place(0, 0)
                }
            },
        cornerRadius = 20.dp,
        onClick = { if (run.done) onDismiss() },
    ) {
        Column(Modifier.padding(start = 18.dp, end = 18.dp, top = 8.dp, bottom = overlap + 8.dp)) {
            Box(
                Modifier.align(Alignment.CenterHorizontally)
                    .size(36.dp, 4.dp)
                    .clip(RoundedCornerShape(2.dp))
                    .background(MiuixTheme.colorScheme.onSurfaceVariantSummary.copy(alpha = 0.3f))
            )
            val status =
                when (val status = run.status) {
                    is ToolStatus.Exited -> if (status.code == 0) null else "exit ${status.code}"
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
                Modifier.padding(top = 10.dp).heightIn(max = 180.dp).verticalScroll(scroll),
                fontFamily = Mono,
                fontSize = 12.sp,
                lineHeight = 17.sp,
                color = MiuixTheme.colorScheme.onSurface,
            )
        }
    }
}
