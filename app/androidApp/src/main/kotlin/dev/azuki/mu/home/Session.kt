package dev.azuki.mu.home

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onPlaced
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.dp
import dev.azuki.mu.Mu
import dev.azuki.mu.chat.Chat
import dev.azuki.mu.chat.ChatState
import dev.azuki.mu.chat.Destination
import dev.azuki.mu.chat.Draft
import dev.azuki.mu.chat.subtitle
import dev.azuki.mu.chat.title
import dev.azuki.mu.composer.Composer
import dev.azuki.mu.composer.ComposerState
import dev.azuki.mu.design.CircleButton
import dev.azuki.mu.design.Heading
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.TopBar
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.Scaffold
import top.yukonga.miuix.kmp.theme.MiuixTheme

private class Arrival(val from: Destination, val to: Destination) {
    val progress = Animatable(0f)
}

private val Rise = spring(stiffness = Spring.StiffnessMediumLow, visibilityThreshold = 0.001f)

@Composable
fun Session(
    mu: Mu,
    composer: ComposerState,
    onMenu: () -> Unit,
    onAddHost: () -> Unit,
    onPickProject: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val window = LocalWindowInfo.current.containerSize
    val destination = mu.destination
    var arrival by remember { mutableStateOf<Arrival?>(null) }
    val moving = arrival?.takeIf { it.to == destination }
    val shown = listOfNotNull(moving?.from, destination)
    fun rise(page: Destination): Float {
        val progress = moving?.progress?.value?.coerceAtMost(1f) ?: return 0f
        return if (page == destination) progress - 1f else progress
    }

    fun newSession(from: ChatState, speed: Float) {
        mu.newChat(from.host, from.cwd ?: mu.draft.cwdFor(from.host))
        val next = Arrival(from, mu.destination)
        arrival = next
        scope.launch {
            next.progress.animateTo(1f, Rise, speed / window.height)
            if (arrival == next) arrival = null
        }
    }

    Scaffold(
        topBar = {
            TopBar(
                heading = {
                    var bottom by remember { mutableFloatStateOf(0f) }
                    Box(
                        Modifier.onPlaced { bottom = it.boundsInWindow().bottom },
                        contentAlignment = Alignment.Center,
                    ) {
                        for (page in shown) {
                            key(page) {
                                Box(
                                    Modifier.graphicsLayer {
                                        val rise = rise(page)
                                        if (page == destination) {
                                            translationY = -window.height * rise
                                        } else {
                                            translationY = -bottom * rise
                                            alpha = 1f - rise
                                            scaleX = 1f - 0.06f * rise
                                            scaleY = scaleX
                                        }
                                    }
                                ) {
                                    when (page) {
                                        is ChatState -> Heading(page.title(), page.subtitle(mu))
                                        is Draft -> {}
                                    }
                                }
                            }
                        }
                    }
                },
                navigation = { CircleButton(Solar.HamburgerMenuLineDuotone, "Sessions", onMenu) },
            )
        },
        bottomBar = {
            Composer(
                state = composer,
                mu = mu,
                destination = destination,
                onSubmit = { if (!composer.submit(it)) onAddHost() },
                modifier =
                    Modifier.windowInsetsPadding(
                            WindowInsets.navigationBars.union(WindowInsets.ime)
                        )
                        .padding(bottom = 8.dp),
            )
        },
    ) { padding ->
        for (page in shown) {
            key(page) {
                Box(
                    Modifier.fillMaxSize()
                        .graphicsLayer {
                            if (page == destination) translationY = -size.height * rise(page)
                        }
                        .background(MiuixTheme.colorScheme.surface)
                ) {
                    Page(mu, page, padding, composer, onAddHost, onPickProject, ::newSession)
                }
            }
        }
    }
}

@Composable
private fun Page(
    mu: Mu,
    page: Destination,
    padding: PaddingValues,
    composer: ComposerState,
    onAddHost: () -> Unit,
    onPickProject: () -> Unit,
    onNewSession: (from: ChatState, speed: Float) -> Unit,
) {
    when (page) {
        is ChatState ->
            Chat(
                mu,
                page,
                padding,
                sent = composer.sent,
                onRecall = { index, text ->
                    mu.launch {
                        if (page.recall(index, text)) composer.restore(text)
                        else mu.toast("Already sent")
                    }
                },
                onNewSession = { speed -> onNewSession(page, speed) },
            )

        is Draft -> Welcome(mu, onAddHost, onPickProject)
    }
}
