package dev.azuki.mu.android

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.content.edit
import dev.azuki.mu.model.Attachment
import dev.azuki.mu.model.Dictation
import dev.azuki.mu.model.MuException
import dev.azuki.mu.model.Platform
import dev.azuki.mu.model.Prefs
import java.io.ByteArrayOutputStream
import java.io.File
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

// The host takes calls up to 4 MiB, which base64 leaves 3 of.
private const val UPLOAD_LIMIT = 3 * 1024 * 1024
private const val IMAGE_EDGE = 2048
private val SENDABLE_IMAGES = setOf("image/png", "image/jpeg", "image/gif", "image/webp")

// Launchers register while the activity is created, so this must be too.
class AndroidPlatform(private val activity: ComponentActivity) : Platform {
    private val context: Context = activity.applicationContext

    override val prefs: Prefs =
        object : Prefs {
            private val shared = context.getSharedPreferences("mu", Context.MODE_PRIVATE)

            override fun get(key: String): String? = shared.getString(key, null)

            override fun set(key: String, value: String?) = shared.edit { putString(key, value) }
        }

    private var permitted: CompletableDeferred<Boolean>? = null
    private val permission =
        activity.registerForActivityResult(ActivityResultContracts.RequestPermission()) {
            permitted?.complete(it)
        }

    override val dictation: Dictation =
        SpeechDictation(context) { ask(Manifest.permission.RECORD_AUDIO) }

    private var shot: CompletableDeferred<Boolean>? = null
    private val camera =
        activity.registerForActivityResult(ActivityResultContracts.TakePicture()) {
            shot?.complete(it)
        }

    private var picked: CompletableDeferred<List<Uri>>? = null
    private val images =
        activity.registerForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(8)) {
            picked?.complete(it)
        }
    private val documents =
        activity.registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
            picked?.complete(it)
        }

    private suspend fun ask(permission: String): Boolean {
        if (
            ContextCompat.checkSelfPermission(context, permission) ==
                PackageManager.PERMISSION_GRANTED
        )
            return true
        val result = CompletableDeferred<Boolean>().also { permitted = it }
        this.permission.launch(permission)
        return result.await()
    }

    override suspend fun allowCamera(): Boolean = ask(Manifest.permission.CAMERA)

    @Composable
    override fun QrScanner(
        onScan: (String) -> Unit,
        onError: (String) -> Unit,
        modifier: Modifier,
    ) = QrViewfinder(activity, onScan, onError, modifier)

    // Declaring the camera permission for scanning makes the camera app need it too.
    override suspend fun takePhoto(): Attachment? {
        if (!allowCamera()) throw MuException("Taking a photo needs the camera")
        val file = File(File(context.cacheDir, "photos").apply { mkdirs() }, "photo.jpg")
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        val result = CompletableDeferred<Boolean>().also { shot = it }
        camera.launch(uri)
        if (!result.await()) return null
        return withContext(Dispatchers.IO) { read(uri, "photo.jpg").also { file.delete() } }
    }

    override suspend fun pickImages(): List<Attachment> {
        val result = CompletableDeferred<List<Uri>>().also { picked = it }
        images.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
        return withContext(Dispatchers.IO) { result.await().map { read(it, null) } }
    }

    override suspend fun pickFiles(): List<Attachment> {
        val result = CompletableDeferred<List<Uri>>().also { picked = it }
        documents.launch(arrayOf("*/*"))
        return withContext(Dispatchers.IO) { result.await().map { read(it, null) } }
    }

    override suspend fun decodeImage(data: ByteArray, size: Int): ImageBitmap? =
        withContext(Dispatchers.Default) {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
            val edge = minOf(bounds.outWidth, bounds.outHeight)
            if (edge <= 0) return@withContext null
            var sample = 1
            while (edge / (sample * 2) >= size) sample *= 2
            val options = BitmapFactory.Options().apply { inSampleSize = sample }
            BitmapFactory.decodeByteArray(data, 0, data.size, options)?.asImageBitmap()
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
                ?: throw MuException("Cannot read $name")
        if (
            type.startsWith("image/") && (bytes.size > UPLOAD_LIMIT / 2 || type !in SENDABLE_IMAGES)
        ) {
            return Attachment(
                name.substringBeforeLast('.') + ".jpg",
                shrink(uri, bytes),
                image = true,
            )
        }
        if (bytes.size > UPLOAD_LIMIT) throw MuException("$name is over 3 MB")
        return Attachment(name, bytes, image = type.startsWith("image/"))
    }

    // Big photos are smaller as a JPEG no larger than the model looks at anyway.
    private fun shrink(uri: Uri, bytes: ByteArray): ByteArray {
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
                var sample = 1
                while (
                    maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= IMAGE_EDGE
                ) sample *= 2
                BitmapFactory.decodeByteArray(
                    bytes,
                    0,
                    bytes.size,
                    BitmapFactory.Options().apply { inSampleSize = sample },
                ) ?: throw MuException("Cannot read this image")
            }
        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.JPEG, 88, out)
        bitmap.recycle()
        return out.toByteArray()
    }
}
