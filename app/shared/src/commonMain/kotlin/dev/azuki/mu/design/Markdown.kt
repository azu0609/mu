package dev.azuki.mu.design

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mikepenz.markdown.compose.Markdown
import com.mikepenz.markdown.model.DefaultMarkdownColors
import com.mikepenz.markdown.model.DefaultMarkdownTypography
import com.mikepenz.markdown.model.markdownAlertColors
import com.mikepenz.markdown.model.markdownPadding
import top.yukonga.miuix.kmp.theme.MiuixTheme

@Composable
fun Markdown(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = MiuixTheme.colorScheme.onSurface,
    small: Boolean = false,
) {
    val styles = MiuixTheme.textStyles
    val body =
        (if (small) styles.body2 else styles.body1).copy(
            color = color,
            lineHeight = if (small) 20.sp else 24.sp,
        )
    val heading = { size: TextUnit ->
        body.copy(fontSize = size, lineHeight = size * 1.35f, fontWeight = FontWeight.Medium)
    }
    val code = body.copy(fontFamily = Mono, fontSize = 13.sp, lineHeight = 19.sp)
    val surface = MiuixTheme.colorScheme.onSurface
    Markdown(
        content = text,
        colors =
            DefaultMarkdownColors(
                text = color,
                codeBackground = surface.copy(alpha = 0.05f),
                inlineCodeBackground = surface.copy(alpha = 0.07f),
                dividerColor = MiuixTheme.colorScheme.dividerLine,
                tableBackground = surface.copy(alpha = 0.02f),
                alert = markdownAlertColors(MiuixTheme.colorScheme.surface.luminance() < 0.5f),
                highlightText = MiuixTheme.colorScheme.onTertiaryContainer,
                highlightBackground = MiuixTheme.colorScheme.tertiaryContainer,
            ),
        typography =
            DefaultMarkdownTypography(
                h1 = heading(if (small) 18.sp else 22.sp),
                h2 = heading(if (small) 17.sp else 20.sp),
                h3 = heading(if (small) 16.sp else 18.sp),
                h4 = heading(if (small) 15.sp else 17.sp),
                h5 = heading(if (small) 14.sp else 16.sp),
                h6 = heading(if (small) 14.sp else 16.sp),
                text = body,
                code = code,
                inlineCode = body.copy(fontFamily = Mono, fontSize = TextUnit.Unspecified),
                quote = body.plus(SpanStyle(fontStyle = FontStyle.Italic)),
                paragraph = body,
                ordered = body,
                bullet = body,
                list = body,
                textLink = TextLinkStyles(SpanStyle(color = Accent)),
                table = body,
            ),
        modifier = modifier.fillMaxWidth(),
        padding = markdownPadding(block = 4.dp),
        immediate = true,
    )
}
