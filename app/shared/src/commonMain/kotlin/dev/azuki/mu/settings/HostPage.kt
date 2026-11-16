package dev.azuki.mu.settings

import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.Connection
import dev.azuki.mu.Mu
import dev.azuki.mu.api.Snippet
import dev.azuki.mu.design.BackButton
import dev.azuki.mu.design.CircleButton
import dev.azuki.mu.design.DialogButtons
import dev.azuki.mu.design.Mono
import dev.azuki.mu.design.PictogramBadge
import dev.azuki.mu.design.Pictograms
import dev.azuki.mu.design.Presented
import dev.azuki.mu.design.SectionTitle
import dev.azuki.mu.design.SmallIconButton
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.TopBar
import dev.azuki.mu.design.icon
import dev.azuki.mu.label
import top.yukonga.miuix.kmp.basic.BasicComponent
import top.yukonga.miuix.kmp.basic.BasicComponentDefaults
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Scaffold
import top.yukonga.miuix.kmp.basic.SmallTitle
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.basic.TextField
import top.yukonga.miuix.kmp.icon.MiuixIcons
import top.yukonga.miuix.kmp.icon.basic.ArrowRight
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.utils.overScrollVertical
import top.yukonga.miuix.kmp.window.WindowDialog

@Composable
fun HostPage(mu: Mu, id: String, onBack: () -> Unit) {
    val host = mu.host(id)
    val live = mu.live[id]
    val profile = live?.profile
    val online = live?.connection == Connection.Online
    var forgetting by remember { mutableStateOf(false) }
    var describing by remember { mutableStateOf(false) }
    var choosingIcon by remember { mutableStateOf(false) }
    val snippet = remember { Presented<Editing>() }
    val deleting = remember { Presented<String>() }
    val icon = host?.icon
    val description = host?.description.orEmpty()
    val gray = MiuixTheme.colorScheme.onSurfaceVariantSummary

    Scaffold(
        topBar = {
            TopBar(
                title = "",
                navigation = { BackButton(onBack) },
                actions = {
                    CircleButton(
                        Solar.UnlinkBoldDuotone,
                        "Unpair",
                        { forgetting = true },
                        enabled = host != null,
                    )
                },
            )
        }
    ) { padding ->
        LazyColumn(
            Modifier.fillMaxHeight().overScrollVertical(),
            contentPadding =
                PaddingValues(
                    top = padding.calculateTopPadding(),
                    bottom =
                        WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding() +
                            12.dp,
                ),
        ) {
            item {
                Column(
                    Modifier.fillMaxWidth().padding(bottom = 16.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    PictogramBadge(
                        Pictograms.host(icon),
                        onClick = if (online) ({ choosingIcon = true }) else null,
                    )
                    Text(
                        mu.name(id),
                        Modifier.padding(top = 16.dp, start = 24.dp, end = 24.dp),
                        style = MiuixTheme.textStyles.title2,
                        fontWeight = FontWeight.Bold,
                        textAlign = TextAlign.Center,
                    )
                    Row(
                        Modifier.padding(top = 4.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .clickable(enabled = online) { describing = true }
                            .padding(horizontal = 8.dp, vertical = 2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            if (description.isBlank()) "No description set" else "“$description”",
                            style = MiuixTheme.textStyles.body2,
                            color = gray,
                        )
                        if (online)
                            Icon(
                                Solar.AltArrowRightLinear,
                                null,
                                Modifier.padding(start = 4.dp).size(14.dp),
                                tint = gray,
                            )
                    }
                }
            }
            item {
                Card(Modifier.padding(horizontal = 12.dp)) {
                    BasicComponent(
                        title = "IP Address",
                        endActions = {
                            Text(
                                host?.address.orEmpty(),
                                fontFamily = Mono,
                                fontSize = 13.sp,
                                color = gray,
                            )
                        },
                    )
                    if (live != null && live.connection != Connection.Online) {
                        BasicComponent(title = live.connection.label(), summary = live.error)
                    }
                }
            }
            item {
                SectionTitle("Bash Snippets", Modifier.padding(top = 8.dp)) {
                    if (online)
                        SmallIconButton(
                            Solar.AddLinear,
                            "New snippet",
                            { snippet.present(Editing(null)) },
                        )
                }
            }
            item {
                Card(Modifier.padding(horizontal = 12.dp)) {
                    val snippets = profile?.snippets.orEmpty()
                    if (snippets.isEmpty()) {
                        Column(
                            Modifier.fillMaxWidth().padding(vertical = 28.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Icon(
                                Solar.CatBoldDuotone,
                                null,
                                Modifier.size(48.dp),
                                tint = MiuixTheme.colorScheme.primary,
                            )
                            Text(
                                "We have nothing but a cat",
                                Modifier.padding(top = 12.dp),
                                style = MiuixTheme.textStyles.headline1,
                            )
                            Text(
                                "hi~",
                                Modifier.padding(top = 2.dp),
                                style = MiuixTheme.textStyles.footnote1,
                                color = gray,
                            )
                        }
                    }
                    snippets.forEach { SnippetRow(it) { snippet.present(Editing(it)) } }
                }
            }
            item {
                Text(
                    "“Bash Snippets” will appear alongside the prompt box as a suggestion you can quickly invoke in the shell mode.",
                    Modifier.padding(horizontal = 28.dp, vertical = 8.dp),
                    style = MiuixTheme.textStyles.footnote1,
                    color = gray,
                )
            }
            if (!live?.sessions.isNullOrEmpty()) {
                item { SmallTitle("Danger Zone") }
                item {
                    Card(Modifier.padding(horizontal = 12.dp)) {
                        BasicComponent(
                            title = "Delete All Sessions…",
                            titleColor =
                                BasicComponentDefaults.titleColor(
                                    color = MiuixTheme.colorScheme.error
                                ),
                            startAction = {
                                Icon(
                                    Solar.TrashBinTrashBoldDuotone,
                                    null,
                                    Modifier.size(24.dp),
                                    tint = MiuixTheme.colorScheme.error,
                                )
                            },
                            onClick = { deleting.present(id) },
                        )
                    }
                }
            }
        }
    }

    WindowDialog(
        show = forgetting,
        title = "Unpair “${mu.name(id)}”?",
        onDismissRequest = { forgetting = false },
    ) {
        val dismiss = LocalDismissState.current
        DialogButtons(
            confirm = "Unpair",
            destructive = true,
            onCancel = { dismiss?.invoke() },
            onConfirm = {
                dismiss?.invoke()
                onBack()
                mu.forget(id)
            },
        )
    }
    DescriptionDialog(describing, description, onDismiss = { describing = false }) { text ->
        mu.launch { mu.remote(id).setDescription(text) }
    }
    IconPicker(
        choosingIcon,
        icon,
        snippet = false,
        onPick = { name -> mu.launch { mu.remote(id).setIcon(name) } },
        onDismiss = { choosingIcon = false },
    )
    snippet.value?.let { editing ->
        SnippetSheet(
            snippet.show,
            mu,
            id,
            editing.snippet,
            onDismiss = snippet::dismiss,
            onDismissFinished = snippet::finished,
        )
    }
    DeleteSessionsDialog(mu, deleting)
}

private class Editing(val snippet: Snippet?)

@Composable
private fun SnippetRow(snippet: Snippet, onClick: () -> Unit) {
    val pictogram = Pictograms.snippet(snippet.icon)
    BasicComponent(
        onClick = onClick,
        startAction = {
            Icon(
                pictogram.vector,
                null,
                Modifier.size(24.dp),
                tint = pictogram.tint,
            )
        },
        endActions = {
            Image(
                MiuixIcons.Basic.ArrowRight,
                null,
                Modifier.size(10.dp, 16.dp).align(Alignment.CenterVertically),
                colorFilter = ColorFilter.tint(MiuixTheme.colorScheme.onSurfaceVariantActions),
            )
        },
    ) {
        Text(snippet.name, style = MiuixTheme.textStyles.headline1, fontWeight = FontWeight.Medium)
        Text(
            snippet.command,
            Modifier.padding(top = 2.dp),
            fontFamily = Mono,
            fontSize = 12.sp,
            lineHeight = 17.sp,
            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            maxLines = 2,
        )
    }
}

@Composable
private fun DescriptionDialog(
    show: Boolean,
    current: String,
    onDismiss: () -> Unit,
    onSave: (String) -> Unit,
) {
    var text by remember(show) { mutableStateOf(current) }
    WindowDialog(
        show = show,
        title = "Description",
        onDismissRequest = onDismiss,
    ) {
        val dismiss = LocalDismissState.current
        Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
            TextField(
                value = text,
                onValueChange = { text = it },
                label = "Description",
                useLabelAsPlaceholder = true,
                singleLine = true,
            )
            DialogButtons(
                confirm = "Save",
                enabled = text.trim() != current,
                onCancel = { dismiss?.invoke() },
                onConfirm = {
                    onSave(text.trim())
                    dismiss?.invoke()
                },
            )
        }
    }
}
