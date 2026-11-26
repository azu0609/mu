package dev.azuki.mu.chat

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.painter.BitmapPainter
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import com.mikepenz.markdown.model.ImageData
import com.mikepenz.markdown.model.ImageTransformer
import com.mikepenz.markdown.model.ImageWidth
import com.mikepenz.markdown.model.PlaceholderConfig
import dev.azuki.mu.Mu
import dev.azuki.mu.core.MuException

// The images a chat's markdown shows, read from its host. Null is one that
// could not be.
class HostImages(private val mu: Mu, private val host: String, private val cwd: () -> String?) :
    ImageTransformer {
    private val painters = mutableStateMapOf<String, Painter?>()
    private val asked = mutableSetOf<String>()

    private fun path(link: String): String? {
        val path = link.removePrefix("file://")
        return when {
            path.startsWith("/") -> path
            ':' in path.substringBefore('/') -> null
            else -> cwd()?.let { "$it/$path" }
        }
    }

    private fun load(path: String, size: Int) {
        if (!asked.add(path)) return
        mu.launch {
            painters[path] =
                try {
                    mu.platform
                        .decodeImage(mu.remote(host).readImage(path), size)
                        ?.let(::BitmapPainter)
                } catch (_: MuException) {
                    null
                }
        }
    }

    @Composable
    override fun transform(link: String): ImageData? {
        val path = path(link) ?: return null
        val size = LocalWindowInfo.current.containerSize.width
        LaunchedEffect(path) { load(path, size) }
        val painter = painters[path] ?: return null
        return ImageData(painter, Modifier.clip(RoundedCornerShape(12.dp)))
    }

    override fun placeholderConfig(
        link: String,
        density: Density,
        containerSize: Size,
        imageWidth: ImageWidth,
        imageSize: Size,
        imageSizeChanged: ((link: String, Size) -> Unit)?,
    ): PlaceholderConfig {
        val failed = path(link)?.let { it in painters && painters[it] == null } ?: true
        if (failed) return PlaceholderConfig(Size.Zero)
        return super.placeholderConfig(
            link,
            density,
            containerSize,
            imageWidth,
            imageSize,
            imageSizeChanged,
        )
    }
}
