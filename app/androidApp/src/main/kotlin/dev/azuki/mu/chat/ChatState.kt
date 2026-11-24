package dev.azuki.mu.chat

import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.azuki.mu.ChatTarget
import dev.azuki.mu.Mu
import dev.azuki.mu.core.MuException
import dev.azuki.mu.core.Session
import dev.azuki.mu.core.SessionListener
import dev.azuki.mu.core.SessionUpdate
import dev.azuki.mu.wire.Block
import dev.azuki.mu.wire.Model
import dev.azuki.mu.wire.Resume
import dev.azuki.mu.wire.Skill
import dev.azuki.mu.wire.State
import dev.azuki.mu.wire.Status
import kotlinx.coroutines.channels.Channel

@Immutable data class Feedback(val id: Int, val block: Block)

class ChatState(private val mu: Mu, val target: ChatTarget) : Destination {
    override val host
        get() = target.host

    override var cwd by mutableStateOf<String?>(null)
        private set

    var state by mutableStateOf<State?>(null)
        private set

    override var skills by mutableStateOf(emptyList<Skill>())
        private set

    var rows by mutableStateOf(emptyList<Row>())
        private set

    var feedback by mutableStateOf(emptyList<Feedback>())
        private set

    var path by mutableStateOf(emptyList<PathNode>())
        private set

    var connected by mutableStateOf(false)
        private set

    var error by mutableStateOf<String?>(null)
        private set

    val images = HostImages(mu, target.host) { cwd }

    override val running
        get() = state?.status == Status.RUNNING

    override val ready
        get() = connected

    override val model
        get() = state?.model?.name

    override val effort
        get() = state?.model?.effort

    private val transcript = Transcript()
    private val live = mutableListOf<Block>()
    private var notice: Block? = null
    private var epoch: ULong? = null
    private var feedbacks = 0
    private var session: Session? = null

    suspend fun run() {
        // None may be dropped: each update builds on the last.
        val batches = Channel<List<SessionUpdate>>(Channel.UNLIMITED)
        val listener =
            object : SessionListener {
                override fun onUpdate(updates: List<SessionUpdate>) {
                    batches.trySend(updates)
                }
            }
        try {
            val resume = epoch?.let { Resume(it, transcript.size.toULong()) }
            mu.remote(host).openSession(target.session, resume, listener).use { session ->
                this.session = session
                for (batch in batches) apply(batch)
            }
        } catch (e: MuException) {
            error = e.message
        } finally {
            session = null
            connected = false
        }
    }

    private fun apply(batch: List<SessionUpdate>) {
        for (update in batch) {
            when (update) {
                is SessionUpdate.Reset -> {
                    epoch = update.epoch
                    transcript.reset(update.keep.toInt())
                    path = transcript.path
                    live.clear()
                    notice = null
                    cwd = update.cwd
                    state = update.state
                    skills = update.skills
                    connected = true
                    error = null
                }

                is SessionUpdate.State -> {
                    state = update.state
                    if (update.started) feedback = emptyList()
                }

                is SessionUpdate.Committed -> {
                    transcript.commit(update.nodes)
                    path = transcript.path
                    live.clear()
                    notice = null
                }

                is SessionUpdate.Live -> {
                    val index = update.index.toInt()
                    if (index < live.size) live[index] = update.block else live.add(update.block)
                }

                is SessionUpdate.LiveNotice -> notice = update.block
                is SessionUpdate.Feedback ->
                    feedback = (feedback + Feedback(feedbacks++, update.block)).takeLast(20)

                is SessionUpdate.Lost -> {
                    connected = false
                    error = update.reason.message
                }
            }
        }
        rows = transcript.rows(live + listOfNotNull(notice), running)
    }

    fun unread(node: ULong) = node >= (state?.unreadFrom ?: 0u)

    fun read() {
        val session = session ?: return
        if (path.none { it.finished != null && unread(it.id) }) return
        mu.launch { runCatching { session.markRead() } }
    }

    private fun connected(): Session =
        session ?: throw MuException.Failed("Not connected to ${mu.name(host)}")

    suspend fun send(text: String, skill: String?) {
        read()
        connected().send(text, skill)
    }

    override suspend fun stop(children: Boolean) = connected().stop(children)

    suspend fun appendShell(run: String) = connected().appendShell(run)

    suspend fun recall(index: Int, text: String) = connected().recallQueued(index.toULong(), text)

    suspend fun tree() = connected().tree()

    suspend fun checkout(node: ULong?) = connected().checkout(node)

    suspend fun edit(node: ULong) = connected().edit(node)

    suspend fun switchBranch(node: ULong) = connected().switchBranch(node)

    // A session always runs some model, so there is no unpicking it.
    override fun use(model: String?, effort: String?) {
        if (model != null) mu.launch { connected().setModel(Model(model, effort, null)) }
    }
}
