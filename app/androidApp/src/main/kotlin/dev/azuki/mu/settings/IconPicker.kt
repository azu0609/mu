package dev.azuki.mu.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.azuki.mu.design.Pictograms
import dev.azuki.mu.design.Sheet
import dev.azuki.mu.design.SheetColumn
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme

@Composable
fun IconPicker(
    show: Boolean,
    current: String?,
    snippet: Boolean,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    Sheet(show = show, title = "Set Icon", onDismissRequest = onDismiss) { padding ->
        val dismiss = LocalDismissState.current
        SheetColumn(padding) {
            Text(
                "Solar Icons by 480 Design",
                Modifier.fillMaxWidth().padding(bottom = 4.dp),
                style = MiuixTheme.textStyles.footnote1,
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                textAlign = TextAlign.Center,
            )
            Pictograms.all.chunked(4).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    row.forEach { pictogram ->
                        val selected = pictogram.name == current
                        Box(
                            Modifier.weight(1f)
                                .aspectRatio(1f)
                                .clip(CircleShape)
                                .background(
                                    if (selected) MiuixTheme.colorScheme.tertiaryContainer
                                    else MiuixTheme.colorScheme.surfaceContainer
                                )
                                .clickable {
                                    onPick(pictogram.name)
                                    dismiss?.invoke()
                                },
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(
                                pictogram.vector,
                                null,
                                Modifier.size(36.dp),
                                tint =
                                    if (snippet) MiuixTheme.colorScheme.primary else pictogram.tint,
                            )
                        }
                    }
                }
            }
        }
    }
}
