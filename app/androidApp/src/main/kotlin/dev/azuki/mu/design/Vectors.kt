package dev.azuki.mu.design

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

internal fun icon(name: String, build: ImageVector.Builder.() -> Unit): ImageVector =
    ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply(build).build()

internal fun ImageVector.Builder.fill(path: String, alpha: Float = 1f, evenOdd: Boolean = false) {
    addPath(
        pathData = addPathNodes(path),
        pathFillType = if (evenOdd) PathFillType.EvenOdd else PathFillType.NonZero,
        fill = SolidColor(Color.Black),
        fillAlpha = alpha,
    )
}

internal fun ImageVector.Builder.stroke(
    path: String,
    alpha: Float = 1f,
    width: Float = 1.5f,
    cap: StrokeCap = StrokeCap.Round,
    join: StrokeJoin = StrokeJoin.Miter,
) {
    addPath(
        pathData = addPathNodes(path),
        stroke = SolidColor(Color.Black),
        strokeAlpha = alpha,
        strokeLineWidth = width,
        strokeLineCap = cap,
        strokeLineJoin = join,
    )
}
