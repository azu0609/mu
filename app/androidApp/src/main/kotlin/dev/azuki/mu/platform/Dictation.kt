package dev.azuki.mu.platform

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import dev.azuki.mu.core.MuException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.withTimeoutOrNull

// The platform's speech recognizer. It must be driven from the main thread.
class Dictation(private val context: Context, private val permit: suspend () -> Boolean) {
    // Loudness, 0 to 1.
    val level = MutableStateFlow(0f)
    val partial = MutableStateFlow("")

    private var recognizer: SpeechRecognizer? = null
    private var result: CompletableDeferred<String>? = null

    // False when the device cannot listen, or may not.
    suspend fun start(): Boolean {
        if (!SpeechRecognizer.isRecognitionAvailable(context)) {
            throw MuException.Failed("This device has no speech recognition to dictate with")
        }
        // Asking interrupts the hold; the next one dictates.
        if (!permit()) return false
        reset()
        val heard = CompletableDeferred<String>().also { result = it }
        recognizer =
            SpeechRecognizer.createSpeechRecognizer(context).apply {
                setRecognitionListener(Listener(heard))
                startListening(
                    Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
                        .putExtra(
                            RecognizerIntent.EXTRA_LANGUAGE_MODEL,
                            RecognizerIntent.LANGUAGE_MODEL_FREE_FORM,
                        )
                        .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
                )
            }
        return true
    }

    suspend fun finish(): String {
        val recognizer = recognizer ?: return ""
        recognizer.stopListening()
        val text = withTimeoutOrNull(4_000) { result?.await() } ?: partial.value
        reset()
        return text
    }

    fun cancel() {
        recognizer?.cancel()
        reset()
    }

    private fun reset() {
        recognizer?.destroy()
        recognizer = null
        result = null
        level.value = 0f
        partial.value = ""
    }

    private inner class Listener(private val heard: CompletableDeferred<String>) :
        RecognitionListener {
        override fun onRmsChanged(rmsdB: Float) {
            level.value = ((rmsdB + 2f) / 12f).coerceIn(0f, 1f)
        }

        override fun onPartialResults(results: Bundle) {
            best(results)?.let { partial.value = it }
        }

        override fun onResults(results: Bundle) {
            heard.complete(best(results) ?: partial.value)
        }

        override fun onError(error: Int) {
            heard.complete(partial.value)
        }

        private fun best(results: Bundle) =
            results
                .getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)
                ?.firstOrNull()
                ?.takeIf { it.isNotBlank() }

        override fun onReadyForSpeech(params: Bundle?) = Unit

        override fun onBeginningOfSpeech() = Unit

        override fun onBufferReceived(buffer: ByteArray?) = Unit

        override fun onEndOfSpeech() = Unit

        override fun onEvent(eventType: Int, params: Bundle?) = Unit
    }
}
