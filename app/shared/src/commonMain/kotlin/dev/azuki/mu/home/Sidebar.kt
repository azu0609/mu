package dev.azuki.mu.home

import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PageSize
import androidx.compose.foundation.pager.PagerDefaults
import androidx.compose.foundation.pager.PagerSnapDistance
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.lerp
import dev.azuki.mu.ChatTarget
import dev.azuki.mu.Connection
import dev.azuki.mu.Mu
import dev.azuki.mu.api.SessionSummary
import dev.azuki.mu.api.Status
import dev.azuki.mu.design.CircleButton
import dev.azuki.mu.design.Dropdown
import dev.azuki.mu.design.Floating
import dev.azuki.mu.design.Glyphs
import dev.azuki.mu.design.SmallIconButton
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.Spinning
import dev.azuki.mu.design.icon
import dev.azuki.mu.design.lifted
import dev.azuki.mu.design.scrim
import dev.azuki.mu.reason
import dev.azuki.mu.tilde
import kotlin.math.absoluteValue
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.BasicComponent
import top.yukonga.miuix.kmp.basic.Button
import top.yukonga.miuix.kmp.basic.ButtonDefaults
import top.yukonga.miuix.kmp.basic.DropdownEntry
import top.yukonga.miuix.kmp.basic.DropdownItem
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

private enum class Filter(
    val icon: ImageVector,
    val tint: @Composable () -> Color,
    val label: String,
    val keep: (SessionSummary) -> Boolean,
) {
    All(Solar.MailboxBoldDuotone, { MiuixTheme.colorScheme.primary }, "All", { true }),
    Running(
        Solar.LoaderBold,
        { MiuixTheme.colorScheme.onSurfaceVariantSummary },
        "In Progress",
        { it.status == Status.Running },
    ),
    Failed(
        Solar.DangerTriangleBoldDuotone,
        { MiuixTheme.colorScheme.error },
        "Failed",
        { it.status == Status.Failed },
    ),
    Finished(
        Solar.CheckCircleLineDuotone,
        { MiuixTheme.colorScheme.primary },
        "Finished",
        { it.status == Status.Finished },
    ),
}

@Composable
fun Sidebar(
    mu: Mu,
    current: ChatTarget?,
    onOpen: (host: String, session: String) -> Unit,
    onNewChat: () -> Unit,
    onNewIn: (host: String) -> Unit,
    onAddHost: () -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val pager = rememberPagerState { Filter.entries.size }
    val filter = Filter.entries[pager.currentPage]
    val hosts = mu.hosts.filter { it.id in mu.selected }

    var bar by remember { mutableIntStateOf(0) }

    Box(modifier.fillMaxSize()) {
        val sides = Modifier.padding(horizontal = 16.dp)
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding =
                PaddingValues(
                    top = WindowInsets.systemBars.asPaddingValues().calculateTopPadding(),
                    bottom = with(LocalDensity.current) { bar.toDp() },
                ),
        ) {
            item(key = "carousel") { Carousel(pager) }
            if (hosts.isEmpty()) {
                item {
                    Empty(
                        if (mu.hosts.isEmpty()) "No hosts yet. Add one below."
                        else "No hosts selected.",
                        sides,
                    )
                }
            }
            hosts.forEach { host ->
                val live = mu.live[host.id]
                val sessions = nested(live?.sessions.orEmpty().filter(filter.keep))
                item(key = "header.${host.id}") {
                    HostHeader(
                        mu = mu,
                        host = host.id,
                        enabled = mu.online(host.id),
                        onNew = { onNewIn(host.id) },
                        modifier = sides,
                    )
                }
                val reason = live?.connection?.reason
                when {
                    reason != null ->
                        item(key = "empty.${host.id}") { Empty(reason, sides, rounded = true) }

                    sessions.isEmpty() && live?.connection == Connection.Online ->
                        item(key = "empty.${host.id}") {
                            Empty(
                                if (filter == Filter.All) "No sessions yet."
                                else "Nothing ${filter.label.lowercase()}.",
                                sides,
                                rounded = true,
                            )
                        }

                    else ->
                        itemsIndexed(
                            sessions,
                            key = { _, (session, _) -> "${host.id}/${session.id}" },
                        ) { index, (session, depth) ->
                            SessionRow(
                                session = session,
                                depth = depth,
                                home = live?.profile?.home,
                                first = index == 0,
                                last = index == sessions.lastIndex,
                                current = current?.host == host.id && current.session == session.id,
                                onClick = { onOpen(host.id, session.id) },
                                modifier = sides,
                            )
                        }
                }
            }
        }
        Box(
            Modifier.align(Alignment.TopCenter)
                .fillMaxWidth()
                .scrim(edgeToEdge = true)
                .windowInsetsPadding(WindowInsets.systemBars.only(WindowInsetsSides.Top))
                .height(16.dp)
        )
        Row(
            Modifier.align(Alignment.BottomCenter)
                .onSizeChanged { bar = it.height }
                .scrim(top = false)
                .navigationBarsPadding()
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            HostsButton(mu, onAddHost, onSettings)
            Spacer(Modifier.weight(1f))
            CircleButton(Solar.AddLinear, "New session", onNewChat, size = 48.dp)
        }
    }
}

@Composable
private fun Carousel(pager: PagerState) {
    val scope = rememberCoroutineScope()
    val settle = remember { spring<Float>(dampingRatio = 0.68f, stiffness = 320f) }
    // The pager clips to its height, so it's given room for the selected fill's shadow out of
    // the padding around it. The fill's fading layer clips too, so it gets the same room.
    val room = 12.dp
    BoxWithConstraints(Modifier.fillMaxWidth().padding(top = 24.dp - room, bottom = 12.dp - room)) {
        val page = 92.dp
        HorizontalPager(
            state = pager,
            pageSize = PageSize.Fixed(page),
            contentPadding = PaddingValues(horizontal = (maxWidth - page) / 2),
            flingBehavior =
                PagerDefaults.flingBehavior(
                    state = pager,
                    pagerSnapDistance = PagerSnapDistance.atMost(Filter.entries.size),
                    snapAnimationSpec = settle,
                ),
            modifier = Modifier.fillMaxWidth().height(page + room * 2),
        ) { index ->
            val filter = Filter.entries[index]
            val tint = filter.tint()
            // How close the page is to the middle, 0 to 1.
            val near = {
                val distance =
                    ((pager.currentPage - index) + pager.currentPageOffsetFraction).absoluteValue
                (1f - distance).coerceIn(0f, 1f)
            }
            Box(
                Modifier.size(page)
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                    ) {
                        scope.launch { pager.animateScrollToPage(index, animationSpec = settle) }
                    }
                    .graphicsLayer {
                        scaleX = 0.75f + 0.25f * near()
                        scaleY = scaleX
                    },
                contentAlignment = Alignment.Center,
            ) {
                Box(
                    Modifier.requiredSize(88.dp + room * 2)
                        .graphicsLayer { alpha = near() }
                        .padding(room)
                        .lifted(CircleShape)
                        .clip(CircleShape)
                        .background(MiuixTheme.colorScheme.surfaceContainer)
                )
                Icon(
                    filter.icon,
                    filter.label,
                    Modifier.size(lerp(44.dp, 52.dp, near())),
                    tint = tint,
                )
            }
        }
    }
}

@Composable
private fun HostHeader(
    mu: Mu,
    host: String,
    enabled: Boolean,
    onNew: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier.fillMaxWidth().padding(start = 12.dp, top = 12.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            mu.name(host),
            Modifier.weight(1f),
            style = MiuixTheme.textStyles.subtitle,
            color = MiuixTheme.colorScheme.onBackgroundVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        SmallIconButton(
            Solar.AddLinear,
            "New session on ${mu.name(host)}",
            onNew,
            enabled = enabled,
        )
    }
}

// Each session, then the ones it spawned beneath it, with how deep each sits. One whose parent
// isn't
// listed, as when filtered out, stands on its own.
private fun nested(sessions: List<SessionSummary>): List<Pair<SessionSummary, Int>> {
    val listed = sessions.mapTo(HashSet()) { it.id }
    val spawned = sessions.groupBy { session -> session.parent?.takeIf { it in listed } }
    val nested = mutableListOf<Pair<SessionSummary, Int>>()
    fun visit(session: SessionSummary, depth: Int) {
        nested += session to depth
        spawned[session.id].orEmpty().forEach { visit(it, depth + 1) }
    }
    spawned[null].orEmpty().forEach { visit(it, 0) }
    return nested
}

@Composable
private fun SessionRow(
    session: SessionSummary,
    depth: Int,
    home: String?,
    first: Boolean,
    last: Boolean,
    current: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val corner = 20.dp
    val shape =
        RoundedCornerShape(
            topStart = if (first) corner else 0.dp,
            topEnd = if (first) corner else 0.dp,
            bottomStart = if (last) corner else 0.dp,
            bottomEnd = if (last) corner else 0.dp,
        )
    BasicComponent(
        modifier = modifier.clip(shape).background(MiuixTheme.colorScheme.surfaceContainer),
        insideMargin =
            PaddingValues(
                start = 16.dp + 20.dp * (depth - 1).coerceIn(0, 3),
                top = 16.dp,
                end = 16.dp,
                bottom = 16.dp,
            ),
        onClick = onClick,
        holdDownState = current,
        startAction =
            if (depth == 0) null
            else {
                {
                    Icon(
                        Glyphs.Spawned,
                        "Subagent",
                        Modifier.size(18.dp),
                        tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                    )
                }
            },
        endActions = { StatusIcon(session.status, session.unread, Modifier.size(22.dp)) },
    ) {
        Text(
            tilde(session.cwd, home),
            style = MiuixTheme.textStyles.footnote2,
            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        Text(
            session.title.ifBlank { "Untitled" },
            Modifier.padding(top = 2.dp),
            style = MiuixTheme.textStyles.headline1,
            fontWeight = FontWeight.Medium,
        )
        session.error?.let {
            Text(
                it,
                Modifier.padding(top = 4.dp),
                style = MiuixTheme.textStyles.footnote1,
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun Empty(text: String, modifier: Modifier = Modifier, rounded: Boolean = false) {
    Text(
        text,
        modifier
            .fillMaxWidth()
            .then(
                if (rounded)
                    Modifier.clip(RoundedCornerShape(20.dp))
                        .background(MiuixTheme.colorScheme.surfaceContainer)
                else Modifier
            )
            .padding(16.dp),
        style = MiuixTheme.textStyles.body2,
        color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
        textAlign = if (rounded) TextAlign.Start else TextAlign.Center,
    )
}

@Composable
private fun HostsButton(mu: Mu, onAddHost: () -> Unit, onSettings: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    val selected = mu.hosts.filter { it.id in mu.selected }
    val label =
        when {
            mu.hosts.isEmpty() -> "Add a Host"
            selected.size == 1 -> mu.name(selected.single().id)
            selected.isEmpty() -> "No Hosts Selected"
            else -> "${selected.size} Hosts Selected"
        }
    Box {
        Floating {
            Button(
                onClick = { if (mu.hosts.isEmpty()) onAddHost() else open = true },
                cornerRadius = 24.dp,
                minHeight = 48.dp,
                colors = ButtonDefaults.buttonColors(color = Color.Transparent),
                insideMargin =
                    PaddingValues(start = 18.dp, end = 16.dp, top = 12.dp, bottom = 12.dp),
            ) {
                Text(
                    label,
                    style = MiuixTheme.textStyles.body1,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                )
                Icon(
                    Solar.ChevronsUpDownBold,
                    null,
                    Modifier.padding(start = 10.dp).size(16.dp),
                    tint = MiuixTheme.colorScheme.onSurface,
                )
            }
        }
        Dropdown(
            show = open,
            onDismiss = { open = false },
            keepOpen = setOf(0),
            entries =
                listOf(
                    DropdownEntry(
                        mu.hosts.map { host ->
                            DropdownItem(
                                host.name,
                                summary =
                                    host.description.takeIf(String::isNotBlank)?.let { "“$it”" },
                                selected = host.id in mu.selected,
                                onClick = { mu.select(host.id, host.id !in mu.selected) },
                            )
                        }
                    ),
                    DropdownEntry(
                        listOf(
                            DropdownItem(
                                "Add New Host…",
                                onClick = onAddHost,
                            ),
                            DropdownItem(
                                "Settings",
                                onClick = onSettings,
                            ),
                        )
                    ),
                ),
        )
    }
}

@Composable
fun StatusIcon(status: Status, unread: Boolean, modifier: Modifier = Modifier) {
    when {
        status == Status.Running -> Spinning(Solar.LoaderBold, modifier)
        status == Status.Failed ->
            Icon(
                Solar.DangerTriangleBoldDuotone,
                "Failed",
                modifier,
                tint = MiuixTheme.colorScheme.error,
            )

        status == Status.Finished && unread ->
            Icon(
                Solar.CheckCircleLineDuotone,
                "Finished",
                modifier,
                tint = MiuixTheme.colorScheme.primary,
            )

        else -> Spacer(modifier)
    }
}
