package dev.azuki.mu.platform

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.content.edit
import dev.azuki.mu.core.MuException
import java.io.ByteArrayOutputStream
import java.io.File
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private const val IMAGE_EDGE = 2048
private val SENDABLE_IMAGES = setOf("image/png", "image/jpeg", "image/gif", "image/webp")

class Attachment(val name: String, val data: ByteArray, val image: Boolean)

class Prefs(context: Context) {
    private val shared = context.getSharedPreferences("mu", Context.MODE_PRIVATE)

    operator fun get(key: String): String? = shared.getString(key, null)

    operator fun set(key: String, value: String?) = shared.edit { putString(key, value) }
}

// Launchers register while the activity is created, so this must be too.
class Platform(activity: ComponentActivity) {
    private val context: Context = activity.applicationContext

    val prefs = Prefs(context)

    // Whether the system derives a palette from the wallpaper.
    val wallpaperColors = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S

    // What the host lists this device as.
    val device: String =
        Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)
            ?.takeIf(String::isNotBlank) ?: "${Build.MANUFACTURER} ${Build.MODEL}"

    private val permission = Asking(activity, ActivityResultContracts.RequestPermission())

    val dictation = Dictation(context) { ask(Manifest.permission.RECORD_AUDIO) }

    private val camera = Asking(activity, ActivityResultContracts.TakePicture())
    private val images = Asking(activity, ActivityResultContracts.PickMultipleVisualMedia(8))
    private val documents = Asking(activity, ActivityResultContracts.OpenMultipleDocuments())

    private suspend fun ask(permission: String): Boolean =
        ContextCompat.checkSelfPermission(context, permission) ==
            PackageManager.PERMISSION_GRANTED || this.permission.ask(permission)

    // False when the user refuses.
    suspend fun allowCamera(): Boolean = ask(Manifest.permission.CAMERA)

    // Declaring the camera permission for scanning makes the camera app need it too.
    suspend fun takePhoto(): Attachment? {
        if (!allowCamera()) throw MuException.Failed("Taking a photo needs the camera")
        val file = File(File(context.cacheDir, "photos").apply { mkdirs() }, "photo.jpg")
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        if (!camera.ask(uri)) return null
        return withContext(Dispatchers.IO) { read(uri, "photo.jpg").also { file.delete() } }
    }

    suspend fun pickImages(): List<Attachment> =
        read(images.ask(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)))

    suspend fun pickFiles(): List<Attachment> = read(documents.ask(arrayOf("*/*")))

    private suspend fun read(picked: List<Uri>): List<Attachment> =
        withContext(Dispatchers.IO) { picked.map { read(it, null) } }

    // Scaled down to about `size` pixels on its shorter edge; null when it isn't an image.
    suspend fun decodeImage(data: ByteArray, size: Int): ImageBitmap? =
        withContext(Dispatchers.Default) {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
            val edge = minOf(bounds.outWidth, bounds.outHeight)
            if (edge <= 0) return@withContext null
            sampled(data, edge, size)?.asImageBitmap()
        }

    private fun read(uri: Uri, fallbackName: String?): Attachment {
        val resolver = context.contentResolver
        val name =
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            } ?: fallbackName ?: uri.lastPathSegment ?: "file"
        val type = resolver.getType(uri).orEmpty()
        val bytes =
            resolver.openInputStream(uri)?.use { it.readBytes() }
                ?: throw MuException.Failed("Cannot read $name")
        if (type.startsWith("image/") && type !in SENDABLE_IMAGES) {
            return Attachment(
                name.substringBeforeLast('.') + ".jpg",
                toJpeg(uri, bytes),
                image = true,
            )
        }
        return Attachment(name, bytes, image = type.startsWith("image/"))
    }

    // Converted to the most widely readable format, no larger than the model looks at anyway.
    private fun toJpeg(uri: Uri, bytes: ByteArray): ByteArray {
        val bitmap =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                ImageDecoder.decodeBitmap(
                    ImageDecoder.createSource(context.contentResolver, uri)
                ) { decoder, info, _ ->
                    val scale =
                        minOf(1f, IMAGE_EDGE.toFloat() / maxOf(info.size.width, info.size.height))
                    decoder.setTargetSize(
                        (info.size.width * scale).toInt(),
                        (info.size.height * scale).toInt(),
                    )
                    decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                }
            } else {
                val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
                sampled(bytes, maxOf(bounds.outWidth, bounds.outHeight), IMAGE_EDGE)
                    ?: throw MuException.Failed("Cannot read this image")
            }
        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.JPEG, 88, out)
        bitmap.recycle()
        return out.toByteArray()
    }
}

// Decoded at the largest power-of-two reduction that keeps `edge` pixels at `target` or more.
private fun sampled(data: ByteArray, edge: Int, target: Int): Bitmap? {
    var sample = 1
    while (edge / (sample * 2) >= target) sample *= 2
    val options = BitmapFactory.Options().apply { inSampleSize = sample }
    return BitmapFactory.decodeByteArray(data, 0, data.size, options)
}

// Something another activity is asked for, one answer at a time.
private class Asking<I, O>(activity: ComponentActivity, contract: ActivityResultContract<I, O>) {
    private var answer: CompletableDeferred<O>? = null
    private val launcher = activity.registerForActivityResult(contract) { answer?.complete(it) }

    suspend fun ask(input: I): O {
        val answer = CompletableDeferred<O>().also { answer = it }
        launcher.launch(input)
        return answer.await()
    }
}
