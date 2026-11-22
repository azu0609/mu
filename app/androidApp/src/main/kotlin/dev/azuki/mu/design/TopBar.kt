package dev.azuki.mu.design

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.pow
import top.yukonga.miuix.kmp.basic.ScrollBehavior
import top.yukonga.miuix.kmp.basic.Text
import top.yukonga.miuix.kmp.basic.TopAppBar
import top.yukonga.miuix.kmp.basic.TopAppBarDefaults
import top.yukonga.miuix.kmp.theme.MiuixTheme

private val Side = TopAppBarDefaults.NavigationIconPadding

private val Horizontal: WindowInsets
    @Composable
    get() =
        WindowInsets.displayCutout
            .union(WindowInsets.navigationBars)
            .only(WindowInsetsSides.Horizontal)

// A top bar the page scrolls under, fading out over it. miuix's bars clip their slots just below
// the status bar, which shears the shadows off the floating buttons in them, so those are laid over
// the bar here rather than put in it. Given a [collapsing] behavior, it's miuix's large title that
// collapses with it.
@Composable
fun TopBar(
    title: String,
    subtitle: String = "",
    collapsing: ScrollBehavior? = null,
    navigation: @Composable () -> Unit = {},
    actions: @Composable RowScope.() -> Unit = {},
) {
    val scrim = Modifier.scrim()
    if (collapsing != null) LargeBar(title, scrim, collapsing, navigation, actions)
    else SmallBar({ Heading(title, subtitle) }, scrim, navigation, actions)
}

@Composable
fun TopBar(
    heading: @Composable () -> Unit,
    navigation: @Composable () -> Unit = {},
    actions: @Composable RowScope.() -> Unit = {},
) = SmallBar(heading, Modifier.scrim(), navigation, actions)

@Composable
fun Heading(title: String, subtitle: String = "") {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text(
            title,
            fontSize = MiuixTheme.textStyles.title3.fontSize,
            fontWeight = FontWeight.Medium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (subtitle.isNotEmpty()) {
            Text(
                subtitle,
                style = MiuixTheme.textStyles.body2,
                color = MiuixTheme.colorScheme.onSurfaceVariantSummary,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
fun Modifier.scrim(top: Boolean = true, edgeToEdge: Boolean = false): Modifier {
    val color = MiuixTheme.colorScheme.surface
    val density = LocalDensity.current
    val inset =
        if (edgeToEdge) 0
        else if (top) WindowInsets.systemBars.getTop(density)
        else WindowInsets.navigationBars.getBottom(density)
    return drawBehind {
        val solid = inset.toFloat().coerceAtMost(size.height)
        val edge = if (top) solid else size.height - solid
        drawRect(
            color,
            topLeft = Offset(0f, if (top) 0f else edge),
            size = Size(size.width, solid),
        )
        val stops =
            Array(17) { i ->
                val t = i / 16f
                t to color.copy(alpha = color.alpha * (1f - t * t).pow(1.5f))
            }
        drawRect(
            Brush.verticalGradient(*stops, startY = edge, endY = if (top) size.height else 0f),
            topLeft = Offset(0f, if (top) edge else 0f),
            size = Size(size.width, size.height - solid),
        )
    }
}

// miuix's own small bar centers just the title on the buttons, leaving a subtitle hanging below.
@Composable
private fun SmallBar(
    heading: @Composable () -> Unit,
    scrim: Modifier,
    navigation: @Composable () -> Unit,
    actions: @Composable RowScope.() -> Unit,
) {
    Layout(
        content = {
            Box { navigation() }
            Row(verticalAlignment = Alignment.CenterVertically, content = actions)
            Box { heading() }
        },
        modifier =
            Modifier.fillMaxWidth()
                .then(scrim)
                .windowInsetsPadding(Horizontal)
                .windowInsetsPadding(WindowInsets.systemBars.only(WindowInsetsSides.Top))
                .padding(horizontal = Side, vertical = 4.dp),
    ) { measurables, constraints ->
        val loose = constraints.copy(minWidth = 0, minHeight = 0)
        val navigation = measurables[0].measure(loose)
        val actions = measurables[1].measure(loose)
        val inset = maxOf(navigation.width, actions.width) + Side.roundToPx()
        val text =
            measurables[2].measure(
                loose.copy(maxWidth = (constraints.maxWidth - inset * 2).coerceAtLeast(0))
            )
        val width = constraints.maxWidth
        val height =
            maxOf(
                TopAppBarDefaults.SmallTopAppBarCenterHeight.roundToPx() - 8.dp.roundToPx(),
                navigation.height,
                actions.height,
                text.height,
            )
        layout(width, height) {
            navigation.placeRelative(0, (height - navigation.height) / 2)
            actions.placeRelative(width - actions.width, (height - actions.height) / 2)
            text.placeRelative((width - text.width) / 2, (height - text.height) / 2)
        }
    }
}

// miuix's collapsing bar, with room kept in its slots for the buttons laid over them.
@Composable
private fun LargeBar(
    title: String,
    scrim: Modifier,
    scrollBehavior: ScrollBehavior,
    navigation: @Composable () -> Unit,
    actions: @Composable RowScope.() -> Unit,
) {
    val density = LocalDensity.current
    var lead by remember { mutableIntStateOf(0) }
    var trail by remember { mutableIntStateOf(0) }
    fun Int.dp(): Dp = with(density) { toDp() }
    Box(scrim) {
        TopAppBar(
            title = title,
            color = Color.Transparent,
            navigationIcon = { Spacer(Modifier.width(lead.dp())) },
            actions = { Spacer(Modifier.width(trail.dp())) },
            scrollBehavior = scrollBehavior,
        )
        Row(
            Modifier.fillMaxWidth()
                .windowInsetsPadding(Horizontal)
                .windowInsetsPadding(WindowInsets.systemBars.only(WindowInsetsSides.Top))
                .height(TopAppBarDefaults.CollapsedHeight)
                .padding(horizontal = Side),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.onSizeChanged { lead = it.width }) { navigation() }
            Spacer(Modifier.weight(1f))
            Row(
                Modifier.onSizeChanged { trail = it.width },
                verticalAlignment = Alignment.CenterVertically,
                content = actions,
            )
        }
    }
}
