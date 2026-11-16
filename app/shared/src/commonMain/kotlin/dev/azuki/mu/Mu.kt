package dev.azuki.mu

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.azuki.mu.api.Core
import dev.azuki.mu.api.Host
import dev.azuki.mu.api.HostProfile
import dev.azuki.mu.api.HostUpdate
import dev.azuki.mu.api.ModelInfo
import dev.azuki.mu.api.MuException
import dev.azuki.mu.api.Remote
import dev.azuki.mu.api.SessionSummary
import dev.azuki.mu.chat.ChatState
import dev.azuki.mu.chat.Destination
import dev.azuki.mu.chat.Draft
import dev.azuki.mu.platform.Platform
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.SnackbarHostState

enum class Connection {
    Connecting,
    Online,
    Offline,
    Unpaired,
}

data class HostLive(
    val profile: HostProfile? = null,
    val sessions: List<SessionSummary> = emptyList(),
    val connection: Connection = Connection.Connecting,
    val error: String? = null,
)

data class ChatTarget(val host: String, val session: String)

class Mu(val core: Core, val platform: Platform, private val scope: CoroutineScope) {
    private val prefs = platform.prefs

    var hosts by mutableStateOf(core.hosts())
        private set

    val live = mutableStateMapOf<String, HostLive>()

    var selected by mutableStateOf(prefs[SELECTED]?.lines()?.toSet() ?: hosts.map { it.id }.toSet())
        private set

    var chat by mutableStateOf<ChatState?>(null)
        private set

    val draft = Draft(this)

    // Where the composer's next message goes.
    val destination: Destination
        get() = chat ?: draft

    private val remotes = mutableMapOf<String, Remote>()

    val models = mutableStateMapOf<String, List<ModelInfo>>()

    val snackbar = SnackbarHostState()

    fun host(id: String): Host? = hosts.find { it.id == id }

    fun remote(id: String): Remote = remotes.getOrPut(id) { core.remote(id) }

    fun name(id: String): String = host(id)?.name ?: id

    fun home(id: String): String? = live[id]?.profile?.home

    fun toast(message: String?) {
        scope.launch { snackbar.showSnackbar(message ?: "Something went wrong") }
    }

    // Runs `action`, telling the user when it fails.
    fun launch(action: suspend () -> Unit) {
        scope.launch {
            try {
                action()
            } catch (e: MuException) {
                toast(e.message)
            }
        }
    }

    suspend fun watch(id: String) {
        while (true) {
            try {
                remote(id).watch().collect { update ->
                    val known = live[id] ?: HostLive()
                    live[id] =
                        when (update) {
                            is HostUpdate.Host ->
                                HostLive(update.profile, update.sessions, Connection.Online)
                            HostUpdate.Lost -> known.copy(connection = Connection.Offline)
                        }
                    if (update is HostUpdate.Host) reloadHosts()
                }
            } catch (e: MuException) {
                live[id] =
                    (live[id] ?: HostLive()).copy(
                        connection = if (e.unpaired) Connection.Unpaired else Connection.Offline,
                        error = e.message,
                    )
            }
            delay(if (live[id]?.connection == Connection.Unpaired) 60_000 else 5_000)
        }
    }

    private fun reloadHosts() {
        val fresh = core.hosts()
        if (fresh != hosts) hosts = fresh
    }

    suspend fun pair(invite: String): Host {
        val host = core.pair(invite)
        reloadHosts()
        select(host.id, true)
        if (draft.host == null) draft.set(host.id, null)
        return host
    }

    // The session's directory, or the host's home when none is picked.
    suspend fun resolveCwd(host: String, cwd: String?): String =
        cwd ?: home(host) ?: remote(host).listDirectory("~").path

    fun forget(id: String) {
        try {
            core.forget(id)
        } catch (e: MuException) {
            toast(e.message)
            return
        }
        live.remove(id)
        reloadHosts()
        select(id, false)
        if (chat?.host == id) chat = null
        if (draft.host == id) draft.set(hosts.firstOrNull()?.id, null)
    }

    fun select(id: String, on: Boolean) {
        selected = if (on) selected + id else selected - id
        prefs[SELECTED] = selected.joinToString("\n")
    }

    fun open(host: String, session: String) {
        val target = ChatTarget(host, session)
        if (chat?.target != target) chat = ChatState(this, target)
    }

    fun newChat(
        host: String? = draft.host,
        cwd: String? = if (host == draft.host) draft.cwd else null,
    ) {
        chat = null
        draft.set(host, cwd)
    }

    // Keeps the last list when the host can't say, as many endpoints don't.
    suspend fun loadModels(host: String) {
        try {
            models[host] = remote(host).listModels()
        } catch (_: MuException) {}
    }

    private companion object {
        const val SELECTED = "hosts.selected"
    }
}

fun Connection.label(): String =
    when (this) {
        Connection.Connecting -> "Connecting"
        Connection.Online -> "Online"
        Connection.Offline -> "Offline"
        Connection.Unpaired -> "Unpaired"
    }
