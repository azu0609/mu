package dev.azuki.mu.platform

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import kotlinx.coroutines.flow.StateFlow

// What only the device itself can do.
interface Platform {
    val prefs: Prefs

    val dictation: Dictation

    // False when the user refuses.
    suspend fun allowCamera(): Boolean

    // Reports the first QR code it sees, once.
    @Composable
    fun QrScanner(onScan: (String) -> Unit, onError: (String) -> Unit, modifier: Modifier)

    suspend fun takePhoto(): Attachment?

    suspend fun pickImages(): List<Attachment>

    suspend fun pickFiles(): List<Attachment>

    // Scaled down to about `size` pixels on its shorter edge; null when it isn't an image.
    suspend fun decodeImage(data: ByteArray, size: Int): ImageBitmap?
}

interface Prefs {
    operator fun get(key: String): String?

    operator fun set(key: String, value: String?)
}

class Attachment(val name: String, val data: ByteArray, val image: Boolean)

interface Dictation {
    // Loudness, 0 to 1.
    val level: StateFlow<Float>

    val partial: StateFlow<String>

    // False when the device cannot listen, or may not.
    suspend fun start(): Boolean

    suspend fun finish(): String

    fun cancel()
}
