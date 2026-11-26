package dev.azuki.mu.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import dev.azuki.mu.design.CircleButton
import dev.azuki.mu.design.PictogramBadge
import dev.azuki.mu.design.Pictograms
import dev.azuki.mu.design.PrimaryButton
import dev.azuki.mu.design.Sheet
import dev.azuki.mu.design.SheetColumn
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.color
import dev.azuki.mu.wire.Pictogram
import top.yukonga.miuix.kmp.basic.ColorPalette
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.InputField
import top.yukonga.miuix.kmp.basic.SearchBar
import top.yukonga.miuix.kmp.basic.SmallTitle
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.utils.overScrollVertical

// Picking an icon takes it in the tint at hand; the palette applies a tint to the current icon.
@Composable
fun IconPicker(
    show: Boolean,
    current: Pictogram?,
    default: String,
    onPick: (Pictogram) -> Unit,
    onDismiss: () -> Unit,
) {
    val name = current?.name ?: default
    var tint by remember(current) { mutableStateOf(current?.tint) }
    var coloring by remember { mutableStateOf(false) }
    var query by remember { mutableStateOf("") }
    val draft = Pictogram(name, tint)
    Sheet(
        show = show,
        title = if (coloring) "Set Color" else "Set Icon",
        onDismissRequest = onDismiss,
        onDismissFinished = {
            tint = current?.tint
            coloring = false
            query = ""
        },
        onBack = if (coloring) ({ coloring = false }) else null,
        endAction = {
            if (coloring) {
                CircleButton(
                    Solar.RestartBoldDuotone,
                    "Use Accent",
                    { tint = null },
                    enabled = tint != null,
                )
            } else {
                CircleButton(
                    Solar.PaletteBoldDuotone,
                    "Set Color",
                    { coloring = true },
                    tint = draft.color,
                )
            }
        },
        bottomAction =
            if (coloring) {
                {
                    val dismiss = LocalDismissState.current
                    PrimaryButton("Apply") {
                        onPick(draft)
                        dismiss?.invoke()
                    }
                }
            } else null,
    ) { padding ->
        if (coloring) {
            SheetColumn(padding) {
                PictogramBadge(
                    draft,
                    default,
                    Modifier.align(Alignment.CenterHorizontally).padding(vertical = 8.dp),
                    size = 88.dp,
                )
                ColorPalette(
                    color = draft.color,
                    onColorChanged = { tint = it.toArgb().toUInt() },
                    showPreview = false,
                )
            }
        } else {
            IconGrid(padding, draft, query, onQueryChange = { query = it }) { picked ->
                onPick(Pictogram(picked, tint))
            }
        }
    }
}

@Composable
private fun IconGrid(
    padding: PaddingValues,
    current: Pictogram,
    query: String,
    onQueryChange: (String) -> Unit,
    onPick: (String) -> Unit,
) {
    val dismiss = LocalDismissState.current
    val color = current.color
    val matches =
        remember(query) {
            val words = query.lowercase().split(' ', '-').filter(String::isNotEmpty)
            Pictograms.all
                .filter { entry -> words.all { it in entry.words } }
                .groupBy { it.category }
        }
    LazyVerticalGrid(
        GridCells.Fixed(5),
        Modifier.fillMaxSize().overScrollVertical(),
        contentPadding = padding,
        verticalArrangement = Arrangement.spacedBy(12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        line { Search(query, onQueryChange) }
        matches.forEach { (category, entries) ->
            line(category) {
                SmallTitle(category, insideMargin = PaddingValues(start = 12.dp, top = 8.dp))
            }
            items(entries, key = { it.name }) { entry ->
                Box(
                    Modifier.aspectRatio(1f)
                        .clip(CircleShape)
                        .background(
                            if (entry.name == current.name) MiuixTheme.colorScheme.tertiaryContainer
                            else MiuixTheme.colorScheme.surfaceContainer
                        )
                        .clickable {
                            onPick(entry.name)
                            dismiss?.invoke()
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(entry.vector, entry.label, Modifier.fillMaxSize(0.55f), tint = color)
                }
            }
        }
        line {
            Text(
                if (matches.isEmpty()) "No icons match “$query”" else "Solar Icons by 480 Design",
                Modifier.fillMaxWidth().padding(vertical = 8.dp),
                style = MiuixTheme.textStyles.footnote1,
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                textAlign = TextAlign.Center,
            )
        }
    }
}

private fun LazyGridScope.line(key: Any? = null, content: @Composable () -> Unit) {
    item(key, span = { GridItemSpan(maxLineSpan) }) { content() }
}

@Composable
private fun Search(query: String, onQueryChange: (String) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    SearchBar(
        inputField = {
            InputField(
                query = query,
                onQueryChange = onQueryChange,
                onSearch = { expanded = false },
                expanded = expanded,
                onExpandedChange = { expanded = it },
                label = "Search",
            )
        },
        onExpandedChange = { expanded = it },
        insideMargin = DpSize(0.dp, 0.dp),
        expanded = expanded,
    ) {}
}
