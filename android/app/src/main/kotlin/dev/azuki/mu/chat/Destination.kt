package dev.azuki.mu.chat

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.azuki.mu.Mu
import dev.azuki.mu.core.MuException
import dev.azuki.mu.wire.Skill

// Where the composer's next message goes: a session that's open, or the draft of one that the
// first message makes.
sealed interface Destination {
    val host: String?
    val cwd: String?
    val model: String?
    val effort: String?
    val skills: List<Skill>
    val running: Boolean

    val ready: Boolean

    // Where @mentions resolve, once known.
    val directory: String?
        get() = cwd

    fun use(model: String?, effort: String?)

    suspend fun stop(children: Boolean)
}

class Draft(private val mu: Mu) : Destination {
    private val prefs = mu.platform.prefs

    override var host by
        mutableStateOf(
            prefs[HOST]?.takeIf { id -> mu.hosts.any { it.id == id } } ?: mu.hosts.firstOrNull()?.id
        )
        private set

    // The directory picked, null being the host's home.
    override var cwd by mutableStateOf(host?.let { prefs["$CWD$it"] })
        private set

    // What was last picked on the host, a null effort being the endpoint's own default.
    override var model by mutableStateOf(host?.let { prefs["$MODEL$it"] })
        private set

    override var effort by mutableStateOf(host?.let { prefs["$EFFORT$it"] })
        private set

    override var skills by mutableStateOf(emptyList<Skill>())
        private set

    override val running = false

    override val ready
        get() = mu.online(host)

    override val directory
        get() = cwd ?: host?.let(mu::home)

    override fun use(model: String?, effort: String?) {
        this.model = model
        this.effort = effort
        host?.let {
            prefs["$MODEL$it"] = model
            prefs["$EFFORT$it"] = effort
        }
    }

    override suspend fun stop(children: Boolean) = Unit

    fun set(host: String?, cwd: String?) {
        if (host != this.host) {
            model = host?.let { prefs["$MODEL$it"] }
            effort = host?.let { prefs["$EFFORT$it"] }
        }
        this.host = host
        this.cwd = cwd
        prefs[HOST] = host
        if (host != null) prefs["$CWD$host"] = cwd
    }

    fun cwdFor(host: String): String? = prefs["$CWD$host"]

    suspend fun loadSkills() {
        skills = emptyList()
        val host = host ?: return
        val directory = directory ?: return
        skills =
            try {
                mu.remote(host).listSkills(directory)
            } catch (_: MuException) {
                emptyList()
            }
    }

    private companion object {
        const val HOST = "draft.host"
        const val CWD = "draft.cwd."
        const val MODEL = "draft.model."
        const val EFFORT = "draft.effort."
    }
}
