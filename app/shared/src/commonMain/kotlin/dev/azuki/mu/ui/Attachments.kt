package dev.azuki.mu.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.input.OutputTransformation
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.model.Attached
import dev.azuki.mu.model.Core
import dev.azuki.mu.model.MuException
import dev.azuki.mu.model.Resolved
import dev.azuki.mu.model.Written
import dev.azuki.mu.state.Destination
import dev.azuki.mu.state.Mu
import dev.azuki.mu.state.baseName
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import top.yukonga.miuix.kmp.basic.HorizontalDivider
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.MiuixTheme

val TileSize = 60.dp

// A file the message attaches, with every mention of it; `ranges` are in UTF-16 units.
class Tile(
    val file: String,
    val kind: Attached,
    val paths: List<String>,
    val ranges: List<TextRange>,
    val color: Color,
)

fun mention(path: String): String =
    if (path.any { it.isWhitespace() || it == '"' } || path.startsWith('@')) {
        "@\"" + path.replace("\\", "\\\\").replace("\"", "\\\"") + "\""
    } else {
        "@$path"
    }

// Mentions are only colored once they attach something, and not while still being typed.
fun tiles(
    text: String,
    selection: TextRange,
    written: List<Written>,
    resolved: Map<String, Resolved>,
): List<Tile> {
    class Gathering(val resolved: Resolved) {
        val paths = mutableListOf<String>()
        val ranges = mutableListOf<TextRange>()
    }
    val gathered = LinkedHashMap<String, Gathering>()
    for (mention in written) {
        if (!mention.closed) continue
        val found = resolved[mention.path] ?: continue
        val start = utf16(text, mention.start)
        val end = utf16(text, mention.end)
        val typing =
            selection.collapsed &&
                selection.start == end &&
                (end == text.length || !text[end].isWhitespace())
        if (typing) continue
        gathered
            .getOrPut(found.file) { Gathering(found) }
            .apply {
                paths += mention.path
                ranges += TextRange(start, end)
            }
    }
    var images = 0
    var texts = 0
    return gathered.values.map {
        val kind = it.resolved.kind
        val color =
            when (kind) {
                Attached.Image -> hue(80f, 90f, images++)
                is Attached.Text -> hue(200f, 100f, texts++)
            }
        Tile(it.resolved.file, kind, it.paths, it.ranges, color)
    }
}

// Each kind keeps to its own stretch of hues, and every next one lands as far from the rest as
// it can.
private fun hue(from: Float, span: Float, index: Int): Color {
    var n = index
    var step = 0.5f
    var at = 0f
    while (n > 0) {
        if (n and 1 == 1) at += step
        n = n shr 1
        step /= 2
    }
    return Color.hsv(from + span * at, 0.62f, 0.78f)
}

fun coloring(core: Core, resolved: Map<String, Resolved>) = OutputTransformation {
    val text = asCharSequence().toString()
    tiles(text, selection, core.mentions(text), resolved).forEach { tile ->
        tile.ranges.forEach { addStyle(SpanStyle(color = tile.color), it.start, it.end) }
    }
}

// The host decides what a mention attaches, so each edit asks it again, keeping what it last said
// meanwhile.
@Composable
fun rememberResolved(
    state: ComposerState,
    mu: Mu,
    destination: Destination,
): Map<String, Resolved> {
    val host = destination.host
    val directory = destination.directory ?: host?.let(mu::home)
    var resolved by remember(host, directory) { mutableStateOf(emptyMap<String, Resolved>()) }
    val skills by rememberUpdatedState(destination.skills)
    LaunchedEffect(state, host, directory) {
        snapshotFlow { state.shell to state.field.text.toString() }
            .collectLatest { (shell, text) ->
                if (host == null || directory == null || shell || '@' !in text) {
                    resolved = emptyMap()
                    return@collectLatest
                }
                delay(150)
                val word = text.trim().takeIf { it.startsWith("/") }?.substringBefore(' ')?.drop(1)
                val skill = skills.firstOrNull { it.name == word }?.name
                try {
                    resolved =
                        mu.core.resolveMentions(host, directory, text, skill).associateBy {
                            it.path
                        }
                } catch (_: MuException) {}
            }
    }
    return resolved
}

@Composable
fun AttachmentStrip(state: ComposerState, mu: Mu, host: String, text: String, tiles: List<Tile>) {
    val cursor = state.field.selection
    Column {
        Row(
            Modifier.horizontalScroll(rememberScrollState())
                .padding(start = 12.dp, end = 12.dp, top = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            tiles.forEach { tile ->
                val active =
                    cursor.collapsed && tile.ranges.any { cursor.start in it.start..it.end }
                AttachmentTile(
                    state,
                    mu,
                    host,
                    tile,
                    active,
                    onClick = {
                        state.field.edit { selection = TextRange(tile.ranges.first().start) }
                        state.focusRequested = true
                    },
                    onRemove = {
                        if (state.field.text.toString() != text) return@AttachmentTile
                        state.field.edit {
                            tile.ranges
                                .sortedByDescending { it.start }
                                .forEach { range ->
                                    val now = asCharSequence()
                                    var start = range.start
                                    var end = range.end
                                    if (end < now.length && now[end] == ' ') end++
                                    else if (start > 0 && now[start - 1] == ' ') start--
                                    replace(start, end, "")
                                }
                        }
                    },
                )
            }
        }
        HorizontalDivider(
            Modifier.padding(start = 18.dp, end = 18.dp, top = 10.dp),
            thickness = 1.dp,
        )
    }
}

@Composable
private fun AttachmentTile(
    state: ComposerState,
    mu: Mu,
    host: String,
    tile: Tile,
    active: Boolean,
    onClick: () -> Unit,
    onRemove: () -> Unit,
) {
    val preview =
        state.previews[tile.file] ?: tile.paths.firstNotNullOfOrNull { state.previews[it] }
    if (tile.kind == Attached.Image && preview == null) {
        val size = with(LocalDensity.current) { TileSize.roundToPx() }
        LaunchedEffect(tile.file) {
            try {
                val data = mu.core.readImage(host, tile.file)
                mu.platform.decodeImage(data, size)?.let { state.previews[tile.file] = it }
            } catch (_: MuException) {}
        }
    }
    val shape = RoundedCornerShape(14.dp)
    Box {
        Box(
            Modifier.padding(top = 9.dp, end = 9.dp)
                .size(TileSize)
                .clip(shape)
                .background(tile.color.copy(alpha = 0.12f))
                .border(if (active) 3.dp else 2.dp, tile.color, shape)
                .clickable(onClick = onClick)
        ) {
            when (val kind = tile.kind) {
                Attached.Image ->
                    if (preview != null) {
                        Image(
                            preview,
                            baseName(tile.file),
                            Modifier.fillMaxSize(),
                            contentScale = ContentScale.Crop,
                        )
                    } else {
                        Icon(
                            Solar.AlbumBoldDuotone,
                            baseName(tile.file),
                            Modifier.align(Alignment.Center).size(26.dp),
                            tint = tile.color,
                        )
                    }

                is Attached.Text ->
                    Column(Modifier.padding(horizontal = 7.dp, vertical = 6.dp)) {
                        Text(
                            baseName(tile.file).substringAfterLast('.', "txt").uppercase(),
                            color = tile.color,
                            fontSize = 10.sp,
                            fontWeight = FontWeight.Bold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Text(
                            kind.head,
                            fontFamily = Mono,
                            fontSize = 5.sp,
                            lineHeight = 6.5.sp,
                            color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                            softWrap = false,
                            overflow = TextOverflow.Clip,
                        )
                    }
            }
        }
        // A ring of the composer's own color cuts the badge out of whatever the tile shows.
        Box(
            Modifier.align(Alignment.TopEnd)
                .size(26.dp)
                .clip(CircleShape)
                .clickable(onClick = onRemove)
                .semantics { contentDescription = "Remove" },
            contentAlignment = Alignment.Center,
        ) {
            Box(
                Modifier.size(22.dp)
                    .background(MiuixTheme.colorScheme.surfaceContainer, CircleShape)
                    .padding(2.dp)
                    .background(tile.color, CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                val ink = lerp(tile.color, Color.Black, 0.55f)
                Canvas(Modifier.size(7.dp)) {
                    val stroke = 1.75.dp.toPx()
                    drawLine(
                        ink,
                        Offset.Zero,
                        Offset(size.width, size.height),
                        stroke,
                        StrokeCap.Round,
                    )
                    drawLine(
                        ink,
                        Offset(size.width, 0f),
                        Offset(0f, size.height),
                        stroke,
                        StrokeCap.Round,
                    )
                }
            }
        }
    }
}
