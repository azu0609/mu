package dev.azuki.mu.chat

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.azuki.mu.ChatTarget
import dev.azuki.mu.Mu
import dev.azuki.mu.api.Block
import dev.azuki.mu.api.Model
import dev.azuki.mu.api.MuException
import dev.azuki.mu.api.Session
import dev.azuki.mu.api.SessionState
import dev.azuki.mu.api.SessionUpdate
import dev.azuki.mu.api.Skill
import dev.azuki.mu.api.Status
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

class ChatState(private val mu: Mu, val target: ChatTarget) : Destination {
    override val host
        get() = target.host

    override var cwd by mutableStateOf<String?>(null)
        private set

    var state by mutableStateOf<SessionState?>(null)
        private set

    override var skills by mutableStateOf(emptyList<Skill>())
        private set

    val blocks = mutableStateListOf<Block>()
    val live = mutableStateListOf<Block>()
    var liveNotice by mutableStateOf<Block?>(null)
        private set

    val feedback = mutableStateListOf<Block>()
    var connected by mutableStateOf(false)
        private set

    var error by mutableStateOf<String?>(null)
        private set

    override val running
        get() = state?.status == Status.Running

    override val model
        get() = state?.model?.name

    override val effort
        get() = state?.model?.effort

    private var session: Session? = null

    suspend fun run() {
        while (true) {
            try {
                mu.remote(host).openSession(target.session).use { session ->
                    this.session = session
                    error = null
                    coroutineScope {
                        session.updates.collect { update ->
                            apply(update)
                            if (update is SessionUpdate.Reset || update is SessionUpdate.State) {
                                if (!running) launch { runCatching { session.markRead() } }
                            }
                        }
                    }
                }
            } catch (e: MuException) {
                error = e.message
            } finally {
                session = null
                connected = false
            }
            delay(3_000)
        }
    }

    private fun apply(update: SessionUpdate) {
        when (update) {
            is SessionUpdate.Reset -> {
                while (blocks.size > update.keep) blocks.removeAt(blocks.lastIndex)
                live.clear()
                liveNotice = null
                cwd = update.cwd
                state = update.state
                skills = update.skills
                connected = true
            }
            is SessionUpdate.State -> {
                state = update.state
                if (update.started) feedback.clear()
            }
            is SessionUpdate.Committed -> {
                blocks.addAll(update.blocks)
                live.clear()
                liveNotice = null
            }
            is SessionUpdate.Live -> {
                if (update.index < live.size) live[update.index] = update.block
                else live.add(update.block)
            }
            is SessionUpdate.LiveNotice -> liveNotice = update.block
            is SessionUpdate.Feedback -> {
                feedback.add(update.block)
                if (feedback.size > 20) feedback.removeAt(0)
            }
            SessionUpdate.Lost -> connected = false
        }
    }

    private fun connected(): Session =
        session ?: throw MuException("Not connected to ${mu.name(host)}")

    suspend fun send(text: String, skill: String?) = connected().send(text, skill)

    override suspend fun stop(children: Boolean) = connected().stop(children)

    suspend fun appendShell(run: String) = connected().appendShell(run)

    suspend fun recall(index: Int, text: String) = connected().recallQueued(index, text)

    // A session always runs some model, so there is no unpicking it.
    override fun use(model: String?, effort: String?) {
        if (model != null) mu.launch { connected().setModel(Model(model, effort)) }
    }
}
