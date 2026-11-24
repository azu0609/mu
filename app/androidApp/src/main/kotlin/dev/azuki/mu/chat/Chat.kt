package dev.azuki.mu.chat

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.snapshots.SnapshotStateMap
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
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.Mu
import dev.azuki.mu.core.compact
import dev.azuki.mu.core.stamp
import dev.azuki.mu.design.CircleButton
import dev.azuki.mu.design.Glyphs
import dev.azuki.mu.design.LocalImages
import dev.azuki.mu.design.Markdown
import dev.azuki.mu.design.Mono
import dev.azuki.mu.design.SmallIconButton
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.Spinning
import dev.azuki.mu.duration
import dev.azuki.mu.ending
import dev.azuki.mu.wire.Block
import dev.azuki.mu.wire.Kind
import dev.azuki.mu.wire.ToolStatus
import kotlin.time.Clock
import kotlinx.coroutines.delay
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

@Composable
fun Chat(
    mu: Mu,
    chat: ChatState,
    padding: PaddingValues,
    sent: Int,
    onRecall: (index: Int, text: String) -> Unit,
    onNewSession: (speed: Float) -> Unit,
) {
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val density = LocalDensity.current
    val release by rememberUpdatedState(onNewSession)
    val pull =
        remember(chat) {
            with(density) {
                PullToNew(
                    scope,
                    threshold = 120.dp.toPx(),
                    travel = 360.dp.toPx(),
                    speed = 1600.dp.toPx(),
                    onArmed = { armed ->
                        if (armed) {
                            haptics.performHapticFeedback(
                                HapticFeedbackType.GestureThresholdActivate
                            )
                        }
                    },
                    onRelease = { speed ->
                        haptics.performHapticFeedback(HapticFeedbackType.Confirm)
                        release(speed)
                    },
                )
            }
        }
    val expanded = remember(chat) { mutableStateMapOf<String, Boolean>() }
    val list = remember(chat) { LazyListState() }
    var follow by remember(chat) { mutableStateOf(true) }
    LaunchedEffect(sent) { follow = true }
    LaunchedEffect(list) {
        snapshotFlow { list.isScrollInProgress }
            .collect { scrolling -> if (!scrolling) follow = !list.canScrollForward }
    }
    val onExpand = { follow = false }
    val behind by remember(list) { derivedStateOf { list.canScrollForward } }

    val state = chat.state
    val rows = chat.rows
    val feedback = chat.feedback
    val queued = state?.queued.orEmpty()
    val error = chat.error?.takeIf { !chat.connected }
    val end = rows.size + feedback.size + queued.size + if (error != null) 1 else 0
    val following = follow
    SideEffect {
        if (list.isScrollInProgress) return@SideEffect
        if (following) return@SideEffect list.requestScrollToItem(end)
        // A row folded away would leave the list wherever its index now points.
        val first = list.layoutInfo.visibleItemsInfo.firstOrNull()?.key as? Int ?: return@SideEffect
        if (rows.any { it.key == first }) return@SideEffect
        val fold = rows.indexOfLast { it.at <= first }
        if (fold >= 0) list.requestScrollToItem(fold)
    }

    Box(
        Modifier.fillMaxSize().pointerInput(chat) {
            awaitEachGesture {
                awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                chat.read()
            }
        }
    ) {
        CompositionLocalProvider(LocalImages provides chat.images) {
            LazyColumn(
                state = list,
                modifier =
                    Modifier.fillMaxSize().nestedScroll(pull).graphicsLayer {
                        val gone = (pull.flight / pull.travel).coerceIn(0f, 1f)
                        translationY = -(pull.offset + pull.flight)
                        alpha = 1f - gone
                        scaleX = 1f - 0.06f * gone
                        scaleY = scaleX
                    },
                contentPadding =
                    PaddingValues(
                        top = padding.calculateTopPadding() + 8.dp,
                        bottom = padding.calculateBottomPadding(),
                        start = 16.dp,
                        end = 16.dp,
                    ),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                items(rows, key = { it.key }, contentType = { it::class }) { row ->
                    if (row is Row.Ended) {
                        EndedView(stamp(row.finished), chat.unread(row.node))
                    } else {
                        RowView(row, expanded, onExpand) { session ->
                            mu.open(chat.target.host, session)
                        }
                    }
                }
                items(feedback, key = { "feedback.${it.id}" }) { NoticeView(it.block) }
                if (error != null) {
                    item(key = "error") { NoticeView(Block(Kind.WARNING, error, null, null, null)) }
                }
                itemsIndexed(queued, key = { index, _ -> "queued.$index" }) { index, text ->
                    QueuedBubble(text, onRecall = { onRecall(index, text) })
                }
                item(key = "end") { Spacer(Modifier.height(2.dp)) }
            }
        }
        PullIndicator(
            pull,
            Modifier.align(Alignment.BottomCenter)
                .padding(bottom = padding.calculateBottomPadding() + 16.dp),
        )
        AnimatedVisibility(
            !follow && behind,
            Modifier.align(Alignment.BottomCenter)
                .padding(bottom = padding.calculateBottomPadding() + 12.dp),
            enter = fadeIn() + scaleIn(),
            exit = fadeOut() + scaleOut(),
        ) {
            CircleButton(Solar.AltArrowDownLinear, "Jump to the end", { follow = true })
        }
    }
}

fun ChatState.title() = state?.title?.ifBlank { null } ?: "New Session"

fun ChatState.subtitle(mu: Mu): String {
    val state = state
    return when {
        !connected -> if (error != null) "Reconnecting to ${mu.name(host)}…" else "Connecting…"
        state == null -> ""
        else ->
            buildString {
                val stats = state.stats
                append(
                    "in ${compact(stats.uncachedInput)} (${compact(stats.input)}) / ${compact(stats.output)} out"
                )
                val context = state.model.context
                if (context != null) append(" / ${stats.contextUsed * 100u / context}% ctx")
                else append(" / ${compact(stats.contextUsed)} ctx")
            }
    }
}

@Composable
private fun RowView(
    row: Row,
    expanded: SnapshotStateMap<String, Boolean>,
    onExpand: () -> Unit,
    onOpen: (session: String) -> Unit,
) {
    when (row) {
        is Row.User -> UserBubble(row.text)
        is Row.AgentMessage -> AgentMessageBubble(row, onOpen)
        is Row.Agent -> Markdown(row.text)
        is Row.Thought -> Step(row.block, row.at, expanded, onExpand)
        is Row.Call -> Step(row.block, row.at, expanded, onExpand)
        is Row.Work -> WorkView(row, expanded, onExpand)
        is Row.Working -> WorkingView(row)
        is Row.Shell -> ShellCard(row.block, onExpand)
        is Row.Notice -> NoticeView(row.block)
        is Row.Ended -> Unit
    }
}

@Composable
private fun EndedView(stamp: String, unread: Boolean) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            stamp,
            Modifier.weight(1f),
            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            style = MiuixTheme.textStyles.footnote2,
        )
        if (unread) {
            Box(Modifier.size(7.dp).clip(CircleShape).background(MiuixTheme.colorScheme.primary))
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
                .background(MiuixTheme.colorScheme.primary)
                .padding(horizontal = 16.dp, vertical = 11.dp),
            color = MiuixTheme.colorScheme.onPrimary,
            style = MiuixTheme.textStyles.body1,
        )
    }
}

@Composable
private fun AgentMessageBubble(row: Row.AgentMessage, onOpen: (session: String) -> Unit) {
    val from = row.from
    Column(
        Modifier.padding(end = 48.dp)
            .clip(RoundedCornerShape(22.dp))
            .background(MiuixTheme.colorScheme.surfaceContainer)
            .then(if (from != null) Modifier.clickable { onOpen(from.session) } else Modifier)
            .padding(horizontal = 16.dp, vertical = 11.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Glyphs.Spawned,
                null,
                Modifier.padding(end = 6.dp).size(14.dp),
                tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            )
            Text(
                from?.title?.ifBlank { null } ?: "Another session",
                style = MiuixTheme.textStyles.footnote1,
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Markdown(row.text, Modifier.padding(top = 4.dp))
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
private fun WorkingView(row: Row.Working) {
    val now by
        produceState(Clock.System.now().toEpochMilliseconds()) {
            while (true) {
                delay(1_000)
                value = Clock.System.now().toEpochMilliseconds()
            }
        }
    Row(Modifier.padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        Spinning(Solar.LoaderBold, Modifier.padding(end = 8.dp).size(16.dp))
        Text(
            row.started?.let { "Working for ${duration(now - it.toLong())}" } ?: "Working…",
            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            style = MiuixTheme.textStyles.body2,
        )
    }
}

@Composable
private fun WorkView(
    work: Row.Work,
    expanded: SnapshotStateMap<String, Boolean>,
    onExpand: () -> Unit,
) {
    val gray = MiuixTheme.colorScheme.onSurfaceVariantSummary
    val steps =
        work.steps.withIndex().filter { (_, block) ->
            block.kind == Kind.CALL || block.text.isNotBlank()
        }
    val open = expanded[work.key] == true
    Column {
        Row(
            Modifier.clip(RoundedCornerShape(8.dp))
                .clickable(enabled = steps.isNotEmpty()) {
                    expanded[work.key] = !open
                    if (!open) onExpand()
                }
                .padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                if (work.started != null && work.finished != null)
                    "Worked for ${duration((work.finished - work.started).toLong())}"
                else "Worked",
                color = gray,
                style = MiuixTheme.textStyles.body2,
            )
            if (steps.isNotEmpty()) {
                Icon(
                    if (open) Solar.AltArrowUpLinear else Solar.AltArrowDownLinear,
                    null,
                    Modifier.padding(start = 6.dp).size(16.dp),
                    tint = gray,
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
                steps.forEach { (index, block) -> Step(block, work.at + index, expanded, onExpand) }
            }
        }
    }
}

@Composable
private fun Step(
    block: Block,
    at: Int,
    expanded: SnapshotStateMap<String, Boolean>,
    onExpand: () -> Unit,
) {
    val key = "step.$at"
    val open = expanded[key] == true
    val toggle = {
        expanded[key] = !open
        if (!open) onExpand()
    }
    when (block.kind) {
        Kind.THOUGHT -> ThoughtView(block, open, toggle)
        Kind.CALL -> CallView(block, open, toggle)
        Kind.AGENT -> Markdown(block.text, small = true)
        Kind.SHELL -> ShellCard(block, onExpand)
        Kind.NOTICE,
        Kind.WARNING -> NoticeView(block)

        Kind.USER,
        Kind.AGENT_MESSAGE -> Unit
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
            .clickable(enabled = !tool?.output?.text.isNullOrEmpty(), onClick = onToggle)
            .padding(horizontal = 14.dp, vertical = 10.dp)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Solar.FileTerminalBoldDuotone,
                null,
                Modifier.size(18.dp),
                tint = MiuixTheme.colorScheme.primary,
            )
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
                        Text(
                            it,
                            color = MiuixTheme.colorScheme.error,
                            fontSize = 11.sp,
                            fontFamily = Mono,
                        )
                    }
            }
        }
        if (open && tool != null) {
            Text(
                tool.output.text.trimEnd() + if (tool.output.truncated) "\n…" else "",
                Modifier.padding(top = 8.dp),
                fontFamily = Mono,
                fontSize = 11.sp,
                lineHeight = 16.sp,
                color = gray,
            )
        }
    }
}

@Composable
private fun ShellCard(block: Block, onExpand: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    val output = block.tool?.output?.text.orEmpty().trimEnd()
    Card(
        cornerRadius = 20.dp,
        onClick = {
            open = !open
            if (open) onExpand()
        },
    ) {
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
                    Text(
                        it,
                        color = MiuixTheme.colorScheme.error,
                        fontSize = 11.sp,
                        fontFamily = Mono,
                    )
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
    if (block.kind == Kind.WARNING) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
            Icon(
                Solar.DangerTriangleBoldDuotone,
                null,
                Modifier.padding(top = 1.dp, end = 8.dp).size(18.dp),
                tint = MiuixTheme.colorScheme.error,
            )
            Text(
                block.text,
                color = MiuixTheme.colorScheme.error,
                style = MiuixTheme.textStyles.body2,
            )
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
