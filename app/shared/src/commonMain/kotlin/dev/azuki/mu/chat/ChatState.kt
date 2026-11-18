package dev.azuki.mu.chat

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.azuki.mu.ChatTarget
import dev.azuki.mu.Mu
import dev.azuki.mu.api.Block
import dev.azuki.mu.api.Model
import dev.azuki.mu.api.MuException
import dev.azuki.mu.api.Resume
import dev.azuki.mu.api.Session
import dev.azuki.mu.api.SessionState
import dev.azuki.mu.api.SessionUpdate
import dev.azuki.mu.api.Skill
import dev.azuki.mu.api.Status
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Immutable data class Feedback(val id: Int, val block: Block)

class ChatState(private val mu: Mu, val target: ChatTarget) : Destination {
    override val host
        get() = target.host

    override var cwd by mutableStateOf<String?>(null)
        private set

    var state by mutableStateOf<SessionState?>(null)
        private set

    override var skills by mutableStateOf(emptyList<Skill>())
        private set

    var rows by mutableStateOf(emptyList<Row>())
        private set

    var feedback by mutableStateOf(emptyList<Feedback>())
        private set

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

    private val transcript = Transcript()
    private val live = mutableListOf<Block>()
    private var notice: Block? = null
    private var epoch: Long? = null
    private var feedbacks = 0
    private var session: Session? = null

    suspend fun run() {
        while (true) {
            try {
                val resume = epoch?.let { Resume(it, transcript.size) }
                mu.remote(host).openSession(target.session, resume).use { session ->
                    this.session = session
                    error = null
                    coroutineScope {
                        session.updates.collect { batch ->
                            apply(batch)
                            val settled = batch.any {
                                it is SessionUpdate.Reset || it is SessionUpdate.State
                            }
                            if (settled && !running) launch { runCatching { session.markRead() } }
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

    private fun apply(batch: List<SessionUpdate>) {
        for (update in batch) {
            when (update) {
                is SessionUpdate.Reset -> {
                    epoch = update.epoch
                    transcript.reset(update.keep)
                    live.clear()
                    notice = null
                    cwd = update.cwd
                    state = update.state
                    skills = update.skills
                    connected = true
                }
                is SessionUpdate.State -> {
                    state = update.state
                    if (update.started) feedback = emptyList()
                }
                is SessionUpdate.Committed -> {
                    transcript.commit(update.blocks)
                    live.clear()
                    notice = null
                }
                is SessionUpdate.Live -> {
                    if (update.index < live.size) live[update.index] = update.block
                    else live.add(update.block)
                }
                is SessionUpdate.LiveNotice -> notice = update.block
                is SessionUpdate.Feedback ->
                    feedback = (feedback + Feedback(feedbacks++, update.block)).takeLast(20)
                SessionUpdate.Lost -> connected = false
            }
        }
        rows = transcript.rows(live + listOfNotNull(notice), running)
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
