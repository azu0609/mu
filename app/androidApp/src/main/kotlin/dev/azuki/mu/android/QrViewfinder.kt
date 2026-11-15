package dev.azuki.mu.android

import androidx.activity.ComponentActivity
import androidx.camera.compose.CameraXViewfinder
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.core.SurfaceRequest
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.lifecycle.awaitInstance
import androidx.camera.viewfinder.core.ImplementationMode
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import java.util.concurrent.Executors
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import zxingcpp.BarcodeReader

@Composable
fun QrViewfinder(
    activity: ComponentActivity,
    onScan: (String) -> Unit,
    onError: (String) -> Unit,
    modifier: Modifier,
) {
    val scan by rememberUpdatedState(onScan)
    val fail by rememberUpdatedState(onError)
    var surface by remember { mutableStateOf<SurfaceRequest?>(null) }
    LaunchedEffect(activity) {
        val decoder = Executors.newSingleThreadExecutor()
        val reader =
            BarcodeReader(BarcodeReader.Options(formats = setOf(BarcodeReader.Format.QR_CODE)))
        val found = Channel<String>(Channel.CONFLATED)
        val preview = Preview.Builder().build().apply { setSurfaceProvider { surface = it } }
        val analysis =
            ImageAnalysis.Builder()
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                .build()
                .apply {
                    setAnalyzer(decoder) { image ->
                        val text = image.use { reader.read(it) }.firstOrNull()?.text
                        if (!text.isNullOrEmpty()) found.trySend(text)
                    }
                }
        var provider: ProcessCameraProvider? = null
        try {
            ProcessCameraProvider.awaitInstance(activity)
                .also { provider = it }
                .bindToLifecycle(
                    activity,
                    CameraSelector.DEFAULT_BACK_CAMERA,
                    preview,
                    analysis,
                )
            scan(found.receive())
            awaitCancellation()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            fail("Cannot scan here: ${e.message ?: "no camera"}")
        } finally {
            provider?.unbind(preview, analysis)
            analysis.clearAnalyzer()
            decoder.shutdown()
        }
    }
    Box(modifier.background(Color.Black)) {
        // A SurfaceView would ignore the sheet's clipping and slide.
        surface?.let {
            CameraXViewfinder(it, Modifier.fillMaxSize(), ImplementationMode.EMBEDDED)
        }
    }
}
