package dev.azuki.mu.ui

import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.lerp
import dev.azuki.mu.model.SessionSummary
import dev.azuki.mu.model.Status
import dev.azuki.mu.state.ChatTarget
import dev.azuki.mu.state.Connection
import dev.azuki.mu.state.Mu
import dev.azuki.mu.state.tilde
import kotlin.math.absoluteValue
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.BasicComponent
import top.yukonga.miuix.kmp.basic.Button
import top.yukonga.miuix.kmp.basic.ButtonDefaults
import top.yukonga.miuix.kmp.basic.DropdownEntry
import top.yukonga.miuix.kmp.basic.DropdownItem
import top.yukonga.miuix.kmp.basic.FloatingToolbar
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.menu.WindowIconDropdownMenu
import top.yukonga.miuix.kmp.theme.MiuixTheme

private enum class Filter(
    val icon: ImageVector,
    val tint: @Composable () -> Color,
    val label: String,
    val keep: (SessionSummary) -> Boolean,
) {
    All(Solar.MailboxBoldDuotone, { Accent }, "All", { true }),
    Running(
        Solar.LoaderBold,
        { MiuixTheme.colorScheme.onSurfaceVariantSummary },
        "In Progress",
        { it.status == Status.Running },
    ),
    Failed(Solar.DangerTriangleBoldDuotone, { Danger }, "Failed", { it.status == Status.Failed }),
    Finished(
        Solar.CheckCircleLineDuotone,
        { Accent },
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
    onHostInfo: (host: String) -> Unit,
    onDeleteSessions: (host: String) -> Unit,
    onAddHost: () -> Unit,
    onSettings: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val pager = rememberPagerState { Filter.entries.size }
    val filter = Filter.entries[pager.currentPage]
    val hosts = mu.hosts.filter { it.id in mu.selected }

    Column(modifier.fillMaxSize().statusBarsPadding()) {
        Carousel(pager)
        LazyColumn(
            Modifier.weight(1f).fillMaxWidth(),
            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
        ) {
            if (hosts.isEmpty()) {
                item {
                    Empty(
                        if (mu.hosts.isEmpty()) "No hosts yet. Add one below."
                        else "No hosts selected."
                    )
                }
            }
            hosts.forEach { host ->
                val live = mu.live[host.id]
                val sessions = live?.sessions.orEmpty().filter(filter.keep)
                item(key = "header.${host.id}") {
                    HostHeader(
                        mu = mu,
                        host = host.id,
                        onNew = { onNewIn(host.id) },
                        onInfo = { onHostInfo(host.id) },
                        onDelete = { onDeleteSessions(host.id) },
                    )
                }
                when {
                    live?.connection == Connection.Offline ||
                        live?.connection == Connection.Unpaired ->
                        item(key = "empty.${host.id}") {
                            Empty(live.error ?: "${live.connection.label()}.", rounded = true)
                        }
                    sessions.isEmpty() && live?.connection == Connection.Online ->
                        item(key = "empty.${host.id}") {
                            Empty(
                                if (filter == Filter.All) "No sessions yet."
                                else "Nothing ${filter.label.lowercase()}.",
                                rounded = true,
                            )
                        }
                    else ->
                        itemsIndexed(
                            sessions,
                            key = { _, session -> "${host.id}/${session.id}" },
                        ) { index, session ->
                            SessionRow(
                                session = session,
                                home = live?.profile?.home,
                                first = index == 0,
                                last = index == sessions.lastIndex,
                                current = current?.host == host.id && current.session == session.id,
                                onClick = { onOpen(host.id, session.id) },
                            )
                        }
                }
            }
        }
        Row(
            Modifier.navigationBarsPadding().padding(16.dp),
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
    // the padding around it.
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
            Box(
                Modifier.size(page)
                    .clickable(
                        interactionSource = remember { MutableInteractionSource() },
                        indication = null,
                    ) {
                        scope.launch { pager.animateScrollToPage(index, animationSpec = settle) }
                    }
                    .graphicsLayer {
                        val distance =
                            ((pager.currentPage - index) + pager.currentPageOffsetFraction)
                                .absoluteValue
                        val near = (1f - distance).coerceIn(0f, 1f)
                        scaleX = 0.75f + 0.25f * near
                        scaleY = scaleX
                    },
                contentAlignment = Alignment.Center,
            ) {
                val distance =
                    ((pager.currentPage - index) + pager.currentPageOffsetFraction).absoluteValue
                val near = (1f - distance).coerceIn(0f, 1f)
                Box(
                    Modifier.size(88.dp)
                        .graphicsLayer { alpha = near }
                        .lifted(CircleShape)
                        .clip(CircleShape)
                        .background(MiuixTheme.colorScheme.surfaceContainer)
                )
                Icon(
                    filter.icon,
                    filter.label,
                    Modifier.size(lerp(44.dp, 52.dp, near)),
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
    onNew: () -> Unit,
    onInfo: () -> Unit,
    onDelete: () -> Unit,
) {
    val description = mu.live[host]?.profile?.description ?: mu.host(host)?.description
    Row(
        Modifier.fillMaxWidth().padding(start = 12.dp, top = 12.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            mu.name(host),
            Modifier.weight(1f),
            style = MiuixTheme.textStyles.subtitle,
            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        SmallIconButton(Solar.AddLinear, "New session on ${mu.name(host)}", onNew)
        WindowIconDropdownMenu(
            entry =
                DropdownEntry(
                    listOf(
                        DropdownItem(
                            "View Info",
                            summary = description?.takeIf(String::isNotBlank)?.let { "“$it”" },
                            icon = {
                                Icon(
                                    Solar.InfoCircleBoldDuotone,
                                    null,
                                    it.size(22.dp),
                                    tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                                )
                            },
                            onClick = onInfo,
                        ),
                        DropdownItem(
                            "Delete All Sessions…",
                            icon = {
                                Icon(
                                    Solar.TrashBinTrashBoldDuotone,
                                    null,
                                    it.size(22.dp),
                                    tint = Danger,
                                )
                            },
                            onClick = onDelete,
                        ),
                    )
                ),
            minWidth = 32.dp,
            minHeight = 32.dp,
        ) {
            Icon(
                Solar.MenuDotsLinear,
                "More",
                Modifier.size(18.dp),
                tint = MiuixTheme.colorScheme.onSurfaceVariantSummary,
            )
        }
    }
}

@Composable
private fun SessionRow(
    session: SessionSummary,
    home: String?,
    first: Boolean,
    last: Boolean,
    current: Boolean,
    onClick: () -> Unit,
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
        modifier = Modifier.clip(shape).background(MiuixTheme.colorScheme.surfaceContainer),
        onClick = onClick,
        holdDownState = current,
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
private fun Empty(text: String, rounded: Boolean = false) {
    Text(
        text,
        Modifier.fillMaxWidth()
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
        FloatingToolbar(outSidePadding = PaddingValues(0.dp)) {
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
                            val description =
                                mu.live[host.id]?.profile?.description ?: host.description
                            DropdownItem(
                                mu.name(host.id),
                                summary = description.takeIf(String::isNotBlank)?.let { "“$it”" },
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
