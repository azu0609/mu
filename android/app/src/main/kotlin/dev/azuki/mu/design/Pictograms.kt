package dev.azuki.mu.design

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.azuki.mu.wire.Pictogram
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.theme.MiuixTheme

const val HostIcon = "solar:server-2-bold-duotone"
const val SnippetIcon = "solar:file-terminal-bold-duotone"

// Solar's bold duotone icons, by the Iconify id hosts and snippets name them with.
object Pictograms {
    class Entry(val name: String, val category: String, private val paths: List<String>) {
        val label = name.removePrefix("solar:").removeSuffix("-bold-duotone").replace('-', ' ')
        val words = "$category $label".lowercase()

        val vector: ImageVector by lazy {
            icon(name) {
                paths.forEach { path ->
                    val (alpha, rule, data) = path.split(' ', limit = 3)
                    fill(data, alpha.toFloat(), evenOdd = rule == "evenodd")
                }
            }
        }
    }

    val all: List<Entry> =
        Pictograms::class.java.getResourceAsStream("pictograms.tsv")!!.bufferedReader().useLines {
            lines ->
            lines
                .map { line ->
                    val fields = line.split('\t')
                    Entry(fields[1], fields[0], fields.drop(2))
                }
                .toList()
        }

    private val byName = all.associateBy(Entry::name)

    // Falls back on `default` for names from icon sets this client lacks.
    fun vector(name: String?, default: String): ImageVector =
        (name?.let(byName::get) ?: byName.getValue(default)).vector
}

val Pictogram?.color: Color
    @Composable
    @ReadOnlyComposable
    get() = this?.tint?.let { Color(it.toInt()) } ?: MiuixTheme.colorScheme.primary

@Composable
fun PictogramIcon(pictogram: Pictogram?, default: String, modifier: Modifier = Modifier) {
    Icon(Pictograms.vector(pictogram?.name, default), null, modifier, tint = pictogram.color)
}

@Composable
fun PictogramBadge(
    pictogram: Pictogram?,
    default: String,
    modifier: Modifier = Modifier,
    size: Dp = 96.dp,
    onClick: (() -> Unit)? = null,
) {
    Box(
        modifier
            .size(size)
            .clip(CircleShape)
            .background(MiuixTheme.colorScheme.surfaceContainer)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier),
        contentAlignment = Alignment.Center,
    ) {
        PictogramIcon(pictogram, default, Modifier.size(size * 0.55f))
    }
}
