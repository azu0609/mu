package dev.azuki.mu.ui

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.AnchoredDraggableState
import androidx.compose.foundation.gestures.DraggableAnchors
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.anchoredDraggable
import androidx.compose.foundation.gestures.animateTo
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.add
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.BlurEffect
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.azuki.mu.model.MuException
import dev.azuki.mu.model.ShellUpdate
import dev.azuki.mu.state.ChatState
import dev.azuki.mu.state.Destination
import dev.azuki.mu.state.Draft
import dev.azuki.mu.state.Mu
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.nav.gesture.PredictiveBackHandler
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.window.WindowDialog

// The home bar takes up some of the gap below the composer, so it gets more room there than it does
// above the keyboard.
private val ComposerInsets: WindowInsets
    @Composable
    get() =
        WindowInsets.navigationBars
            .add(WindowInsets(bottom = 16.dp))
            .union(WindowInsets.ime.add(WindowInsets(bottom = 8.dp)))
            .only(WindowInsetsSides.Bottom)

private enum class Drawer {
    Closed,
    Open,
}

@Composable
fun Home(
    mu: Mu,
    onSettings: () -> Unit,
    onHost: (String) -> Unit,
    invite: String?,
    onInviteHandled: () -> Unit,
) {
    val composer = remember { ComposerState(mu.platform.prefs) }
    val scope = rememberCoroutineScope()
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val density = LocalDensity.current
    var addingHost by remember { mutableStateOf(false) }
    val picking = remember { Presented<String>() }
    val deleting = remember { Presented<String>() }
    val drawer = remember { AnchoredDraggableState(Drawer.Closed) }
    val motion = spring<Float>(dampingRatio = 0.9f, stiffness = 500f)

    fun setDrawer(value: Drawer) {
        scope.launch { drawer.animateTo(value, motion) }
    }

    LaunchedEffect(invite) { if (invite != null) addingHost = true }

    val target = mu.chat
    val chat = target?.let { remember(it) { ChatState(mu, it) } }
    if (chat != null) {
        LaunchedEffect(chat) { lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) { chat.run() } }
    }
    val currentChat by rememberUpdatedState(chat)
    val destination: Destination = chat ?: mu.draft
    if (chat == null) {
        LaunchedEffect(mu.draft.host, mu.draft.directory) { mu.draft.loadSkills() }
    }
    LaunchedEffect(destination.host) { destination.host?.let { mu.loadModels(it) } }
    var sent by remember { mutableIntStateOf(0) }
    var composerHeight by remember { mutableIntStateOf(0) }

    fun runShell(host: String, command: String) {
        val chat = currentChat
        composer.run?.job?.cancel()
        val run = ShellRun(command)
        composer.run = run
        run.job = scope.launch {
            try {
                val cwd = mu.resolveCwd(host, (chat ?: mu.draft).cwd)
                mu.core.shell(host, cwd, command).collect { update ->
                    when (update) {
                        is ShellUpdate.Output -> run.output = update.tool.output
                        is ShellUpdate.Failed -> run.failure = update.message
                        is ShellUpdate.Done -> {
                            run.output = update.tool.output
                            run.status = update.tool.status
                            if (chat != null && chat.blocks.isNotEmpty() && !chat.running) {
                                try {
                                    chat.appendShell(update.run)
                                    if (composer.run === run) composer.run = null
                                } catch (e: MuException) {
                                    mu.toast("Not added to the conversation: ${e.message}")
                                }
                            }
                        }
                    }
                }
            } catch (e: MuException) {
                run.failure = e.message
            }
        }
    }

    fun submit(submission: Submission) {
        val destination = currentChat ?: mu.draft
        val host = destination.host
        if (host == null) {
            addingHost = true
            return
        }
        when (submission) {
            is Submission.Command -> runShell(host, submission.command)
            is Submission.Prompt ->
                scope.launch {
                    composer.busy = true
                    sent++
                    try {
                        when (destination) {
                            is ChatState -> {
                                val text =
                                    mu.withAttachments(
                                        host,
                                        submission.text,
                                        submission.attachments,
                                    )
                                destination.send(text, submission.skill)
                            }
                            is Draft -> {
                                val model =
                                    destination.model ?: throw MuException("Pick a model first")
                                val cwd = mu.resolveCwd(host, destination.cwd)
                                val text =
                                    mu.withAttachments(
                                        host,
                                        submission.text,
                                        submission.attachments,
                                    )
                                val id =
                                    mu.core.createSession(
                                        host,
                                        cwd,
                                        model,
                                        destination.effort,
                                        text,
                                        submission.skill,
                                    )
                                mu.open(host, id)
                            }
                        }
                    } catch (e: MuException) {
                        mu.toast(e.message)
                        if (composer.field.text.isEmpty()) {
                            composer.field.edit { replace(0, length, submission.text) }
                            composer.attachments += submission.attachments
                        }
                    } finally {
                        composer.busy = false
                    }
                }
        }
    }

    val dictationBlur by animateDpAsState(if (composer.dictating) 24.dp else 0.dp)
    val surface = MiuixTheme.colorScheme.surface
    BoxWithConstraints(
        Modifier.fillMaxSize()
            .background(MiuixTheme.colorScheme.surface)
            .anchoredDraggable(drawer, Orientation.Horizontal)
    ) {
        val sidebarWidth = maxWidth * 0.84f
        val width = with(density) { sidebarWidth.toPx() }
        SideEffect {
            drawer.updateAnchors(
                DraggableAnchors {
                    Drawer.Closed at 0f
                    Drawer.Open at width
                }
            )
        }
        val offset = { drawer.offset.takeUnless(Float::isNaN) ?: 0f }
        val open = drawer.currentValue == Drawer.Open || drawer.targetValue == Drawer.Open

        PredictiveBackHandler(
            enabled = open,
            onProgress = { events ->
                drawer.anchoredDrag { events.collect { dragTo(width * (1f - it.progress)) } }
            },
            onCommit = { setDrawer(Drawer.Closed) },
            onCancel = { setDrawer(Drawer.Open) },
        )

        Sidebar(
            mu = mu,
            current = target,
            onOpen = { host, session ->
                if (target?.host != host || target.session != session) mu.open(host, session)
                setDrawer(Drawer.Closed)
            },
            onNewChat = {
                mu.newChat()
                setDrawer(Drawer.Closed)
            },
            onNewIn = { host -> picking.present(host) },
            onHostInfo = onHost,
            onDeleteSessions = { host -> deleting.present(host) },
            onAddHost = { addingHost = true },
            onSettings = onSettings,
            modifier =
                Modifier.width(sidebarWidth).graphicsLayer {
                    val progress = (offset() / width).coerceIn(0f, 1f)
                    translationX = (offset() - width) * 0.3f
                    alpha = progress
                },
        )

        Box(
            Modifier.fillMaxSize()
                .graphicsLayer {
                    val progress = (offset() / width).coerceIn(0f, 1f)
                    translationX = offset()
                    scaleX = 1f - 0.06f * progress
                    scaleY = scaleX
                    transformOrigin = TransformOrigin(0f, 0.5f)
                    shape = RoundedCornerShape(40.dp * progress)
                    clip = progress > 0f
                    shadowElevation = 16.dp.toPx() * progress
                    val blur = maxOf(dictationBlur.toPx(), 12.dp.toPx() * progress)
                    renderEffect = if (blur > 0f) BlurEffect(blur, blur, TileMode.Decal) else null
                }
                .background(surface)
        ) {
            if (chat != null) {
                Chat(
                    mu,
                    chat,
                    bottom = with(density) { composerHeight.toDp() },
                    sent = sent,
                    onMenu = { setDrawer(Drawer.Open) },
                )
            } else {
                Welcome(
                    mu = mu,
                    onMenu = { setDrawer(Drawer.Open) },
                    onAddHost = { addingHost = true },
                    onPickProject = { mu.draft.host?.let(picking::present) },
                )
            }
            Composer(
                state = composer,
                mu = mu,
                destination = destination,
                onSubmit = { submit(it) },
                modifier =
                    Modifier.align(Alignment.BottomCenter)
                        .onSizeChanged { composerHeight = it.height }
                        .windowInsetsPadding(ComposerInsets),
            )
            if (open) {
                Box(
                    Modifier.fillMaxSize().pointerInput(Unit) {
                        detectTapGestures { setDrawer(Drawer.Closed) }
                    }
                )
            }
        }

        DictationOverlay(composer.dictating, composer.cancelArmed, mu.platform.dictation)
    }

    AddHostSheet(
        show = addingHost,
        mu = mu,
        invite = invite,
        onDismiss = {
            addingHost = false
            onInviteHandled()
        },
    )
    picking.value?.let { host ->
        ProjectPicker(
            show = picking.show,
            mu = mu,
            host = host,
            start = if (host == mu.draft.host) mu.draft.cwd else mu.draft.cwdFor(host),
            onPick = { cwd ->
                mu.newChat(host, cwd)
                setDrawer(Drawer.Closed)
            },
            onDismiss = picking::dismiss,
            onDismissFinished = picking::finished,
        )
    }
    DeleteSessionsDialog(mu, deleting)
}

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
                        val deleted = mu.core.deleteSessions(it)
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
