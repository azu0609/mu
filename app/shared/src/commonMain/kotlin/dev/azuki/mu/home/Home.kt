package dev.azuki.mu.home

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.azuki.mu.Mu
import dev.azuki.mu.composer.ComposerState
import dev.azuki.mu.composer.DictationOverlay
import dev.azuki.mu.composer.StopOverlay
import dev.azuki.mu.design.Drawer
import dev.azuki.mu.design.Presented
import dev.azuki.mu.design.rememberDrawerState
import dev.azuki.mu.settings.AddHostSheet

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
    var addingHost by remember { mutableStateOf(false) }
    val picking = remember { Presented<String>() }
    val drawer = rememberDrawerState()

    LaunchedEffect(invite) { if (invite != null) addingHost = true }

    val chat = mu.chat
    if (chat != null) {
        LaunchedEffect(chat) { lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) { chat.run() } }
    } else {
        LaunchedEffect(mu.draft.host, mu.draft.directory, mu.draft.ready) {
            if (mu.draft.ready) mu.draft.loadSkills()
        }
    }
    val host = mu.destination.host
    val online = mu.online(host)
    LaunchedEffect(host, online) { if (host != null && online) mu.loadModels(host) }

    val blur by animateDpAsState(if (composer.dictating || composer.stopMenu.open) 24.dp else 0.dp)
    Box(Modifier.fillMaxSize()) {
        Drawer(
            state = drawer,
            blur = { blur },
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
            Session(
                mu,
                composer,
                onMenu = drawer::open,
                onAddHost = { addingHost = true },
                onPickProject = { mu.draft.host?.takeIf(mu::online)?.let(picking::present) },
            )
        }

        DictationOverlay(composer.dictating, composer.cancelArmed, mu.platform.dictation)
        StopOverlay(composer.stopMenu)
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
