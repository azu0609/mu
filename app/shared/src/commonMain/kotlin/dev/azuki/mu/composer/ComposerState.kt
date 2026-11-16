package dev.azuki.mu.composer

import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.text.TextRange
import dev.azuki.mu.api.ToolStatus
import dev.azuki.mu.platform.Prefs
import kotlinx.coroutines.Job

class ComposerState(private val prefs: Prefs) {
    val field = TextFieldState()
    var keyboard by mutableStateOf(prefs[KEYBOARD] == "1")
        private set

    var shell by mutableStateOf(false)
        private set

    var focusRequested by mutableStateOf(false)
    var snippets by mutableStateOf(false)
    var uploading by mutableIntStateOf(0)
    // Images shown for what is attached, by the file or the path it's mentioned as.
    val previews = mutableStateMapOf<String, ImageBitmap>()
    var busy by mutableStateOf(false)
    var run by mutableStateOf<ShellRun?>(null)
    var dictating by mutableStateOf(false)
    var cancelArmed by mutableStateOf(false)

    fun useKeyboard(on: Boolean) {
        keyboard = on
        focusRequested = on
        prefs[KEYBOARD] = if (on) "1" else null
    }

    fun insert(mention: String) {
        field.edit {
            val text = asCharSequence()
            val at = selection.max
            val lead = if (at > 0 && !text[at - 1].isWhitespace()) " " else ""
            val trail = if (at < text.length && text[at].isWhitespace()) "" else " "
            val inserted = lead + mention + trail
            replace(at, at, inserted)
            selection = TextRange(at + inserted.length)
        }
    }

    // Puts back a message that was sent, after whatever is typed.
    fun restore(text: String) {
        field.edit {
            val typed = asCharSequence().trimEnd()
            val restored = if (typed.isEmpty()) text else "$typed\n$text"
            replace(0, length, restored)
            selection = TextRange(restored.length)
        }
        if (shell) useShell(false) else focusRequested = keyboard
    }

    fun useShell(on: Boolean) {
        shell = on
        snippets = false
        focusRequested = on || keyboard
    }

    private companion object {
        const val KEYBOARD = "composer.keyboard"
    }
}

class ShellRun(val command: String) {
    var output by mutableStateOf("")
    var status by mutableStateOf<ToolStatus>(ToolStatus.Running)
    var failure by mutableStateOf<String?>(null)
    var job: Job? = null
    val done
        get() = status != ToolStatus.Running || failure != null
}

sealed interface Submission {
    data class Prompt(val text: String, val skill: String?) : Submission

    data class Command(val command: String) : Submission
}
