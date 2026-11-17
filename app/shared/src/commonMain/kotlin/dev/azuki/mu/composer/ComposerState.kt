package dev.azuki.mu.composer

import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.text.TextRange
import dev.azuki.mu.Mu
import dev.azuki.mu.api.Model
import dev.azuki.mu.api.MuException
import dev.azuki.mu.api.ShellUpdate
import dev.azuki.mu.api.Skill
import dev.azuki.mu.api.ToolStatus
import dev.azuki.mu.chat.ChatState
import dev.azuki.mu.chat.Draft
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

class ComposerState(private val mu: Mu, private val scope: CoroutineScope) {
    private val prefs = mu.platform.prefs

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
        private set

    // Counts the messages sent.
    var sent by mutableIntStateOf(0)
        private set

    var run by mutableStateOf<ShellRun?>(null)
    var dictating by mutableStateOf(false)
    var cancelArmed by mutableStateOf(false)
    val stopMenu = StopMenu()

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

    // False when there is no host to send to yet.
    fun submit(submission: Submission): Boolean {
        val host = mu.destination.host ?: return false
        when (submission) {
            is Submission.Command -> runShell(host, submission.command)
            is Submission.Prompt -> scope.launch { send(host, submission) }
        }
        return true
    }

    private suspend fun send(host: String, prompt: Submission.Prompt) {
        busy = true
        sent++
        try {
            when (val destination = mu.destination) {
                is ChatState -> destination.send(prompt.text, prompt.skill)
                is Draft -> {
                    val model = destination.model ?: throw MuException("Pick a model first")
                    val id =
                        mu.remote(host)
                            .createSession(
                                mu.resolveCwd(host, destination.cwd),
                                Model(model, destination.effort),
                                prompt.text,
                                prompt.skill,
                            )
                    mu.open(host, id)
                }
            }
        } catch (e: MuException) {
            mu.toast(e.message)
            if (field.text.isEmpty()) field.edit { replace(0, length, prompt.text) }
        } finally {
            busy = false
        }
    }

    // A command run in a session joins its conversation once done, when the agent is idle.
    private fun runShell(host: String, command: String) {
        val chat = mu.chat
        run?.job?.cancel()
        val run = ShellRun(command)
        this.run = run
        run.job = scope.launch {
            try {
                val cwd = mu.resolveCwd(host, mu.destination.cwd)
                mu.remote(host).shell(cwd, command).collect { update ->
                    when (update) {
                        is ShellUpdate.Output -> run.output = update.tool.output
                        is ShellUpdate.Failed -> run.failure = update.message
                        is ShellUpdate.Done -> {
                            run.output = update.tool.output
                            run.status = update.tool.status
                            if (chat != null && chat.blocks.isNotEmpty() && !chat.running) {
                                try {
                                    chat.appendShell(update.run)
                                    if (this@ComposerState.run === run)
                                        this@ComposerState.run = null
                                } catch (e: MuException) {
                                    mu.toast("Not added to the conversation: ${e.message}")
                                }
                            }
                        }
                    }
                }
            } catch (e: MuException) {
                run.failure = e.message
            }
        }
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

// The skill a message invokes, by starting with its /name.
fun invoked(text: String, skills: List<Skill>): String? {
    val word = text.trim().takeIf { it.startsWith("/") }?.substringBefore(' ')?.drop(1)
    return skills.firstOrNull { it.name == word }?.name
}
