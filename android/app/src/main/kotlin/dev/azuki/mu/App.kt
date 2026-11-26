package dev.azuki.mu

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.azuki.mu.core.Core
import dev.azuki.mu.design.MuTheme
import dev.azuki.mu.home.Home
import dev.azuki.mu.platform.Platform
import dev.azuki.mu.settings.HostPage
import dev.azuki.mu.settings.SettingsPage
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable
import top.yukonga.miuix.kmp.basic.SnackbarHost
import top.yukonga.miuix.kmp.nav.core.NavDisplay
import top.yukonga.miuix.kmp.nav.core.NavKey
import top.yukonga.miuix.kmp.nav.core.rememberNavBackStack
import top.yukonga.miuix.kmp.nav.transition.NavSwipeDirection

@Serializable
sealed interface Route : NavKey {
    @Serializable data object Home : Route

    @Serializable data object Settings : Route

    @Serializable data class Host(val id: String) : Route
}

// `invite` is a mu:// link the app was opened with, until `onInviteHandled`.
@Composable
fun App(core: Core, platform: Platform, invite: String?, onInviteHandled: () -> Unit) {
    MuTheme(platform.wallpaperColors) {
        val scope = rememberCoroutineScope()
        val mu = remember { Mu(core, platform, scope) }
        val lifecycle = LocalLifecycleOwner.current.lifecycle
        val ids = mu.hosts.map { it.id }
        LaunchedEffect(ids, mu.pairings) {
            lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
                coroutineScope { ids.forEach { launch { mu.watch(it) } } }
            }
        }

        val backStack = rememberNavBackStack<Route>(Route.Home)
        fun push(route: Route) {
            if (backStack.lastOrNull() != route) backStack.add(route)
        }
        Box(Modifier.fillMaxSize()) {
            NavDisplay(backStack) {
                entry<Route.Home> {
                    Home(
                        mu = mu,
                        onSettings = { push(Route.Settings) },
                        invite = invite,
                        onInviteHandled = onInviteHandled,
                    )
                }
                entry<Route.Settings>(swipeDismiss = NavSwipeDirection.LeftToRight) {
                    SettingsPage(
                        mu,
                        onBack = { backStack.removeLastOrNull() },
                        onHost = { push(Route.Host(it)) },
                    )
                }
                entry<Route.Host>(swipeDismiss = NavSwipeDirection.LeftToRight) { route ->
                    HostPage(mu, route.id, onBack = { backStack.removeLastOrNull() })
                }
            }
            SnackbarHost(
                mu.snackbar,
                Modifier.align(Alignment.BottomCenter)
                    .navigationBarsPadding()
                    .imePadding()
                    .padding(bottom = 96.dp),
            )
        }
    }
}
