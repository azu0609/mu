package dev.azuki.mu.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.state.Mu
import dev.azuki.mu.state.baseName
import dev.azuki.mu.state.tilde
import top.yukonga.miuix.kmp.basic.Button
import top.yukonga.miuix.kmp.basic.ButtonDefaults
import top.yukonga.miuix.kmp.basic.DropdownEntry
import top.yukonga.miuix.kmp.basic.DropdownItem
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

@Composable
fun Welcome(
    mu: Mu,
    onMenu: () -> Unit,
    onAddHost: () -> Unit,
    onPickProject: () -> Unit,
) {
    val host = mu.draft.host
    val cwd = mu.draft.directory

    Box(Modifier.fillMaxSize()) {
        CircleButton(
            Solar.HamburgerMenuLineDuotone,
            "Sessions",
            onMenu,
            Modifier.statusBarsPadding().padding(start = 16.dp, top = 8.dp),
        )
        Column(Modifier.align(Alignment.CenterStart).padding(horizontal = 28.dp)) {
            Text("嗨!", style = MiuixTheme.textStyles.title1, fontWeight = FontWeight.Bold)
            Text(
                "我是mu, 你好呀～",
                style = MiuixTheme.textStyles.title1,
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            )
            Row(
                Modifier.padding(top = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (host == null) {
                    Chip("Add a Host", Solar.AddLinear, onAddHost)
                } else {
                    var picking by remember { mutableStateOf(false) }
                    Box {
                        Chip(mu.name(host), Solar.ChevronsUpDownBold) { picking = true }
                        Dropdown(
                            show = picking,
                            onDismiss = { picking = false },
                            entries =
                                listOf(
                                    DropdownEntry(
                                        mu.hosts.map { known ->
                                            DropdownItem(
                                                mu.name(known.id),
                                                selected = known.id == host,
                                                onClick = {
                                                    mu.draft.set(
                                                        known.id,
                                                        mu.draft.cwdFor(known.id),
                                                    )
                                                },
                                            )
                                        }
                                    ),
                                    DropdownEntry(
                                        listOf(
                                            DropdownItem(
                                                "Add New Host…",
                                                onClick = onAddHost,
                                            )
                                        )
                                    ),
                                ),
                        )
                    }
                    Chip(
                        cwd?.let { if (tilde(it, mu.home(host)) == "~") "~" else baseName(it) }
                            ?: "~",
                        Solar.AltArrowRightLinear,
                        onPickProject,
                    )
                }
            }
        }
    }
}

@Composable
private fun Chip(text: String, icon: ImageVector, onClick: () -> Unit) {
    Floating {
        Button(
            onClick = onClick,
            cornerRadius = 20.dp,
            minHeight = 36.dp,
            colors = ButtonDefaults.buttonColors(color = Color.Transparent),
            insideMargin = PaddingValues(start = 14.dp, end = 12.dp, top = 8.dp, bottom = 8.dp),
        ) {
            Text(
                text,
                fontFamily = Mono,
                fontSize = 14.sp,
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                maxLines = 1,
            )
            Icon(
                icon,
                null,
                Modifier.padding(start = 8.dp).size(14.dp),
                tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            )
        }
    }
}
