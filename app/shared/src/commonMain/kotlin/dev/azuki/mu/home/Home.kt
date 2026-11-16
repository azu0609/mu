package dev.azuki.mu.home

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.add
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.azuki.mu.Mu
import dev.azuki.mu.chat.Chat
import dev.azuki.mu.composer.Composer
import dev.azuki.mu.composer.ComposerState
import dev.azuki.mu.composer.DictationOverlay
import dev.azuki.mu.design.Drawer
import dev.azuki.mu.design.Presented
import dev.azuki.mu.design.rememberDrawerState
import dev.azuki.mu.settings.AddHostSheet
import kotlinx.coroutines.launch

// The home bar takes up some of the gap below the composer, so it gets more room there than it does
// above the keyboard.
private val ComposerInsets: WindowInsets
    @Composable
    get() =
        WindowInsets.navigationBars
            .add(WindowInsets(bottom = 16.dp))
            .union(WindowInsets.ime.add(WindowInsets(bottom = 8.dp)))
            .only(WindowInsetsSides.Bottom)

@Composable
fun Home(
    mu: Mu,
    onSettings: () -> Unit,
    invite: String?,
    onInviteHandled: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val composer = remember { ComposerState(mu, scope) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val density = LocalDensity.current
    var addingHost by remember { mutableStateOf(false) }
    val picking = remember { Presented<String>() }
    val drawer = rememberDrawerState()

    LaunchedEffect(invite) { if (invite != null) addingHost = true }

    val chat = mu.chat
    if (chat != null) {
        LaunchedEffect(chat) { lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) { chat.run() } }
    } else {
        LaunchedEffect(mu.draft.host, mu.draft.directory) { mu.draft.loadSkills() }
    }
    LaunchedEffect(mu.destination.host) { mu.destination.host?.let { mu.loadModels(it) } }
    var composerHeight by remember { mutableIntStateOf(0) }

    val dictationBlur by animateDpAsState(if (composer.dictating) 24.dp else 0.dp)
    Box(Modifier.fillMaxSize()) {
        Drawer(
            state = drawer,
            blur = { dictationBlur },
            sidebar = { modifier ->
                Sidebar(
                    mu = mu,
                    current = chat?.target,
                    onOpen = { host, session ->
                        mu.open(host, session)
                        drawer.close()
                    },
                    onNewChat = {
                        mu.newChat()
                        drawer.close()
                    },
                    onNewIn = { host ->
                        mu.newChat(host, mu.draft.cwdFor(host))
                        drawer.close()
                    },
                    onAddHost = { addingHost = true },
                    onSettings = onSettings,
                    modifier = modifier,
                )
            },
        ) {
            if (chat != null) {
                Chat(
                    mu,
                    chat,
                    bottom = with(density) { composerHeight.toDp() },
                    sent = composer.sent,
                    onMenu = drawer::open,
                    onRecall = { index, text ->
                        mu.launch {
                            if (chat.recall(index, text)) composer.restore(text)
                            else mu.toast("Already sent")
                        }
                    },
                )
            } else {
                Welcome(
                    mu = mu,
                    onMenu = drawer::open,
                    onAddHost = { addingHost = true },
                    onPickProject = { mu.draft.host?.let(picking::present) },
                )
            }
            Composer(
                state = composer,
                mu = mu,
                destination = mu.destination,
                onSubmit = { if (!composer.submit(it)) addingHost = true },
                modifier =
                    Modifier.align(Alignment.BottomCenter)
                        .onSizeChanged { composerHeight = it.height }
                        .windowInsetsPadding(ComposerInsets),
            )
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
                drawer.close()
            },
            onDismiss = picking::dismiss,
            onDismissFinished = picking::finished,
        )
    }
}
