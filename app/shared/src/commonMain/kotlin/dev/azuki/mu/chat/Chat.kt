package dev.azuki.mu.chat

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.snapshots.SnapshotStateMap
import androidx.compose.runtime.structuralEqualityPolicy
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.Mu
import dev.azuki.mu.api.Block
import dev.azuki.mu.api.BlockKind
import dev.azuki.mu.api.ToolStatus
import dev.azuki.mu.design.Accent
import dev.azuki.mu.design.CircleButton
import dev.azuki.mu.design.Danger
import dev.azuki.mu.design.Glyphs
import dev.azuki.mu.design.Markdown
import dev.azuki.mu.design.Mono
import dev.azuki.mu.design.SmallIconButton
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.Spinning
import dev.azuki.mu.design.TopBar
import dev.azuki.mu.duration
import kotlin.time.Clock
import kotlinx.coroutines.delay
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Scaffold
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

@Composable
fun Chat(
    mu: Mu,
    chat: ChatState,
    // How much of the bottom the composer covers.
    bottom: Dp,
    // Counts the messages sent, each one bringing the list back to its end.
    sent: Int,
    onMenu: () -> Unit,
    onRecall: (index: Int, text: String) -> Unit,
) {
    val expanded = remember(chat) { mutableStateMapOf<String, Boolean>() }
    val entries by
        remember(chat) {
            derivedStateOf {
                entries(chat.blocks.toList(), chat.liveNotice, chat.live.toList(), chat.running)
            }
        }
    val state = chat.state
    val list = rememberLazyListState()
    var follow by remember(chat) { mutableStateOf(true) }
    LaunchedEffect(sent) { follow = true }
    LaunchedEffect(list) {
        snapshotFlow { list.isScrollInProgress }
            .collect { scrolling ->
                if (!scrolling)
                    follow =
                        list.firstVisibleItemIndex == 0 && list.firstVisibleItemScrollOffset == 0
            }
    }
    val newest by
        remember(chat) {
            derivedStateOf(structuralEqualityPolicy()) {
                listOf(
                    entries.lastOrNull()?.key,
                    entries.size,
                    chat.feedback.size,
                    chat.state?.queued?.size,
                    chat.liveNotice != null,
                )
            }
        }
    // The list is reversed and holds its first visible item in place, so anything new would land
    // below the fold. Pinning to the end in the same frame the items change keeps it in view.
    key(newest) {
        SideEffect { if (follow && !list.isScrollInProgress) list.requestScrollToItem(0) }
    }

    Scaffold(
        topBar = {
            TopBar(
                title = state?.title?.ifBlank { null } ?: "New Session",
                subtitle =
                    when {
                        !chat.connected ->
                            if (chat.error != null) "Reconnecting to ${mu.name(chat.host)}…"
                            else "Connecting…"
                        state == null -> ""
                        else ->
                            buildString {
                                append(
                                    "in ${mu.core.compact(state.uncachedInput)} (${mu.core.compact(state.input)}) / ${mu.core.compact(state.output)} out"
                                )
                                val context = state.context
                                if (context != null)
                                    append(" / ${state.contextUsed * 100 / context}% ctx")
                                else append(" / ${mu.core.compact(state.contextUsed)} ctx")
                            }
                    },
                navigation = { CircleButton(Solar.HamburgerMenuLineDuotone, "Sessions", onMenu) },
            )
        }
    ) { padding ->
        LazyColumn(
            state = list,
            reverseLayout = true,
            modifier = Modifier.fillMaxSize(),
            contentPadding =
                PaddingValues(
                    top = padding.calculateTopPadding() + 8.dp,
                    bottom = bottom + 16.dp,
                    start = 16.dp,
                    end = 16.dp,
                ),
            verticalArrangement = Arrangement.spacedBy(14.dp, Alignment.Bottom),
        ) {
            val queued = state?.queued.orEmpty()
            queued.asReversed().forEachIndexed { reversed, text ->
                val index = queued.size - 1 - reversed
                item(key = "queued.$index") {
                    QueuedBubble(text, onRecall = { onRecall(index, text) })
                }
            }
            chat.error
                ?.takeIf { !chat.connected }
                ?.let { error ->
                    item(key = "error") {
                        NoticeView(Block(BlockKind.Warning, error, null, null))
                    }
                }
            chat.feedback.asReversed().forEachIndexed { index, block ->
                item(key = "feedback.${chat.feedback.size - 1 - index}") { NoticeView(block) }
            }
            items(entries.asReversed(), key = { it.key }) { entry ->
                when (entry) {
                    is Entry.User -> UserBubble(entry.text)
                    is Entry.Agent -> Markdown(entry.text)
                    is Entry.Work -> WorkView(entry, expanded)
                    is Entry.Shell -> ShellCard(entry.block)
                    is Entry.Notice -> NoticeView(entry.block)
                }
            }
        }
    }
}

@Composable
private fun UserBubble(text: String) {
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
        Text(
            text,
            Modifier.padding(start = 48.dp)
                .clip(RoundedCornerShape(22.dp))
                .background(Accent)
                .padding(horizontal = 16.dp, vertical = 11.dp),
            color = MiuixTheme.colorScheme.onPrimary,
            style = MiuixTheme.textStyles.body1,
        )
    }
}

@Composable
private fun QueuedBubble(text: String, onRecall: () -> Unit) {
    val dots = MiuixTheme.colorScheme.onSurfaceVariantSummary.copy(alpha = 0.5f)
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.End,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        SmallIconButton(Glyphs.Recall, "Back to the composer", onRecall)
        Text(
            text,
            Modifier.padding(start = 4.dp)
                .clip(RoundedCornerShape(22.dp))
                .background(MiuixTheme.colorScheme.surfaceContainer)
                .drawBehind {
                    val width = 2.dp.toPx()
                    val inset = width / 2
                    drawRoundRect(
                        dots,
                        topLeft = Offset(inset, inset),
                        size = Size(size.width - width, size.height - width),
                        cornerRadius = CornerRadius(22.dp.toPx() - inset),
                        style =
                            Stroke(
                                width,
                                cap = StrokeCap.Round,
                                pathEffect =
                                    PathEffect.dashPathEffect(floatArrayOf(0f, 5.dp.toPx())),
                            ),
                    )
                }
                .padding(horizontal = 16.dp, vertical = 11.dp),
            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            style = MiuixTheme.textStyles.body1,
        )
    }
}

@Composable
private fun WorkView(work: Entry.Work, expanded: SnapshotStateMap<String, Boolean>) {
    val now by
        produceState(Clock.System.now().toEpochMilliseconds(), work.ongoing) {
            while (work.ongoing) {
                delay(1_000)
                value = Clock.System.now().toEpochMilliseconds()
            }
        }
    val label =
        when {
            work.ongoing && work.started != null -> "Working for ${duration(now - work.started)}"
            work.ongoing -> "Working…"
            work.started != null && work.finished != null ->
                "Worked for ${duration(work.finished - work.started)}"
            else -> "Worked"
        }
    val gray = MiuixTheme.colorScheme.onSurfaceVariantSummary
    val steps = work.steps.filter { it.block.kind == BlockKind.Call || it.block.text.isNotBlank() }
    val open = expanded[work.key] == true
    Column {
        Row(
            Modifier.clip(RoundedCornerShape(8.dp))
                .clickable(enabled = steps.isNotEmpty()) { expanded[work.key] = !open }
                .padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (work.ongoing) Spinning(Solar.LoaderBold, Modifier.padding(end = 8.dp).size(16.dp))
            Text(label, color = gray, style = MiuixTheme.textStyles.body2)
            if (steps.isNotEmpty()) {
                Icon(
                    if (open) Solar.AltArrowUpLinear else Solar.AltArrowDownLinear,
                    null,
                    Modifier.padding(start = 6.dp).size(16.dp),
                    tint = gray,
                )
            }
        }
        if (work.ongoing && !open) {
            steps.lastOrNull()?.let { last ->
                Text(
                    headline(last.block.text),
                    Modifier.padding(top = 2.dp),
                    color = gray,
                    style = MiuixTheme.textStyles.footnote1,
                    fontFamily = if (last.block.kind == BlockKind.Call) Mono else null,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        AnimatedVisibility(
            open,
            enter = fadeIn() + expandVertically(),
            exit = fadeOut() + shrinkVertically(),
        ) {
            Column(
                Modifier.padding(top = 8.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                steps.forEach { step ->
                    val block = step.block
                    val toggle = { expanded[step.key] = expanded[step.key] != true }
                    when (block.kind) {
                        BlockKind.Thought -> ThoughtView(block, expanded[step.key] == true, toggle)
                        BlockKind.Call -> CallView(block, expanded[step.key] == true, toggle)
                        BlockKind.Agent -> Markdown(block.text, small = true)
                        BlockKind.Shell -> ShellCard(block)
                        BlockKind.Notice,
                        BlockKind.Warning -> NoticeView(block)
                        BlockKind.User -> Unit
                    }
                }
            }
        }
    }
}

private fun headline(text: String): String =
    text.lineSequence().firstOrNull { it.isNotBlank() }.orEmpty().trim().trim('*', '#', ' ')

@Composable
private fun ThoughtView(block: Block, open: Boolean, onToggle: () -> Unit) {
    val gray = MiuixTheme.colorScheme.onSurfaceVariantSummary
    val title = headline(block.text)
    val more = block.text.trim().trim('*', '#', ' ') != title
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .clickable(enabled = more, onClick = onToggle)
    ) {
        if (open) {
            Markdown(block.text, color = gray, small = true)
        } else {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    title,
                    Modifier.weight(1f, fill = false),
                    color = gray,
                    style = MiuixTheme.textStyles.body2.copy(fontStyle = FontStyle.Italic),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (more) {
                    Icon(
                        Solar.AltArrowDownLinear,
                        null,
                        Modifier.padding(start = 6.dp).size(14.dp),
                        tint = gray,
                    )
                }
            }
        }
    }
}

@Composable
private fun CallView(block: Block, open: Boolean, onToggle: () -> Unit) {
    val tool = block.tool
    val gray = MiuixTheme.colorScheme.onSurfaceVariantSummary
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(MiuixTheme.colorScheme.surfaceContainer)
            .clickable(enabled = !tool?.output.isNullOrEmpty(), onClick = onToggle)
            .padding(horizontal = 14.dp, vertical = 10.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Solar.FileTerminalBoldDuotone, null, Modifier.size(18.dp), tint = Accent)
            Text(
                block.text,
                Modifier.weight(1f).padding(horizontal = 8.dp),
                fontFamily = Mono,
                fontSize = 12.sp,
                lineHeight = 17.sp,
                maxLines = if (open) Int.MAX_VALUE else 2,
                overflow = TextOverflow.Ellipsis,
            )
            when (val status = tool?.status) {
                ToolStatus.Running,
                ToolStatus.Pending -> Spinning(Solar.LoaderBold, Modifier.size(14.dp))
                null -> Unit
                else ->
                    ending(status)?.let {
                        Text(it, color = Danger, fontSize = 11.sp, fontFamily = Mono)
                    }
            }
        }
        if (open && tool != null) {
            Text(
                tool.output.trimEnd() + if (tool.truncated) "\n…" else "",
                Modifier.padding(top = 8.dp),
                fontFamily = Mono,
                fontSize = 11.sp,
                lineHeight = 16.sp,
                color = gray,
            )
        }
    }
}

private fun ending(status: ToolStatus): String? =
    when (status) {
        is ToolStatus.Exited -> if (status.code == 0) null else "exit ${status.code}"
        is ToolStatus.TimedOut -> "timed out"
        ToolStatus.Cancelled -> "cancelled"
        ToolStatus.Failed -> "failed"
        ToolStatus.Running,
        ToolStatus.Pending -> null
    }

@Composable
private fun ShellCard(block: Block) {
    var open by remember { mutableStateOf(false) }
    val output = block.tool?.output.orEmpty().trimEnd()
    Card(cornerRadius = 20.dp, onClick = { open = !open }) {
        Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "$ ${block.text}",
                    Modifier.weight(1f),
                    fontFamily = Mono,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Medium,
                )
                block.tool?.status?.let(::ending)?.let {
                    Text(it, color = Danger, fontSize = 11.sp, fontFamily = Mono)
                }
            }
            if (output.isNotEmpty()) {
                Text(
                    output,
                    Modifier.padding(top = 6.dp),
                    fontFamily = Mono,
                    fontSize = 12.sp,
                    lineHeight = 17.sp,
                    color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                    maxLines = if (open) Int.MAX_VALUE else 6,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun NoticeView(block: Block) {
    if (block.kind == BlockKind.Warning) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
            Icon(
                Solar.DangerTriangleBoldDuotone,
                null,
                Modifier.padding(top = 1.dp, end = 8.dp).size(18.dp),
                tint = Danger,
            )
            Text(block.text, color = Danger, style = MiuixTheme.textStyles.body2)
        }
    } else {
        Text(
            block.text,
            Modifier.fillMaxWidth(),
            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            style = MiuixTheme.textStyles.footnote1,
            textAlign = TextAlign.Center,
        )
    }
}
