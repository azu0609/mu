package dev.azuki.mu.settings

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.unit.dp
import dev.azuki.mu.Connection
import dev.azuki.mu.Mu
import dev.azuki.mu.design.BackButton
import dev.azuki.mu.design.Pictograms
import dev.azuki.mu.design.SectionTitle
import dev.azuki.mu.design.SmallIconButton
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.TopBar
import dev.azuki.mu.label
import top.yukonga.miuix.kmp.basic.BasicComponent
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.MiuixScrollBehavior
import top.yukonga.miuix.kmp.basic.Scaffold
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.preference.ArrowPreference
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.utils.overScrollVertical

private const val REPOSITORY = "https://github.com/azu0609/mu"

@Composable
fun SettingsPage(mu: Mu, onBack: () -> Unit, onHost: (String) -> Unit) {
    val behavior = MiuixScrollBehavior()
    val uri = LocalUriHandler.current
    var addingHost by remember { mutableStateOf(false) }
    Scaffold(
        topBar = {
            TopBar(
                title = "Settings",
                collapsing = behavior,
                navigation = { BackButton(onBack) },
            )
        }
    ) { padding ->
        LazyColumn(
            Modifier.fillMaxHeight()
                .overScrollVertical()
                .nestedScroll(behavior.nestedScrollConnection),
            contentPadding =
                PaddingValues(
                    top = padding.calculateTopPadding(),
                    bottom =
                        WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding() +
                            12.dp,
                ),
        ) {
            item {
                SectionTitle("Hosts") {
                    SmallIconButton(Solar.AddLinear, "Add host", { addingHost = true })
                }
            }
            item {
                Card(Modifier.padding(horizontal = 12.dp)) {
                    if (mu.hosts.isEmpty()) {
                        BasicComponent(
                            title = "No hosts yet",
                            summary = "Pair one to start a session on it.",
                        )
                    }
                    mu.hosts.forEach { host ->
                        val pictogram = Pictograms.host(host.icon)
                        ArrowPreference(
                            title = mu.name(host.id),
                            summary = host.description.takeIf(String::isNotBlank)?.let { "“$it”" },
                            startAction = {
                                Icon(
                                    pictogram.vector,
                                    null,
                                    Modifier.size(28.dp),
                                    tint = pictogram.tint,
                                )
                            },
                            endActions = {
                                Text(
                                    (mu.live[host.id]?.connection ?: Connection.Connecting).label(),
                                    style = MiuixTheme.textStyles.body2,
                                    color = MiuixTheme.colorScheme.onSurfaceVariantActions,
                                )
                            },
                            onClick = { onHost(host.id) },
                        )
                    }
                }
            }
            item {
                Card(Modifier.padding(12.dp)) {
                    ArrowPreference(
                        title = "关于mu",
                        endActions = {
                            Text(
                                mu.core.version,
                                style = MiuixTheme.textStyles.body2,
                                color = MiuixTheme.colorScheme.onSurfaceVariantActions,
                            )
                        },
                        onClick = { uri.openUri(REPOSITORY) },
                    )
                }
            }
        }
    }
    AddHostSheet(addingHost, mu, onDismiss = { addingHost = false })
}
