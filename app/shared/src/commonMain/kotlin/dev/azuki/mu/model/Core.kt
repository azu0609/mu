package dev.azuki.mu.model

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

// Everything that talks to hosts. Calls throw MuException.
interface Core {
    val version: String

    fun hosts(): List<Host>

    suspend fun pair(invite: String): Host

    fun forget(host: String)

    // Each update is all of it; the flow keeps going through losses.
    fun watch(host: String): Flow<HostUpdate>

    // Starts the session with its first message.
    suspend fun createSession(
        host: String,
        cwd: String,
        model: String,
        effort: String?,
        text: String,
        skill: String?,
    ): String

    suspend fun listModels(host: String): List<ModelInfo>

    suspend fun deleteSessions(host: String): Deleted

    suspend fun setIcon(host: String, icon: String?)

    suspend fun setDescription(host: String, description: String)

    suspend fun saveSnippet(
        host: String,
        id: String?,
        name: String,
        command: String,
        icon: String?,
    ): String

    suspend fun deleteSnippet(host: String, id: String)

    suspend fun listDirectory(host: String, path: String): Directory

    suspend fun listSkills(host: String, cwd: String): List<Skill>

    // Where the host keeps the file, for a message to @mention.
    suspend fun upload(host: String, name: String, data: ByteArray): String

    fun shell(host: String, cwd: String, command: String): Flow<ShellUpdate>

    suspend fun openSession(host: String, session: String): Session
}

interface Session : AutoCloseable {
    // None may be dropped: each update builds on the last.
    val updates: Flow<SessionUpdate>

    suspend fun send(text: String, skill: String?)

    suspend fun appendShell(run: String)

    suspend fun setModel(name: String, effort: String?)

    suspend fun stop()

    suspend fun markRead()
}

// What only the device itself can do.
interface Platform {
    val prefs: Prefs

    val dictation: Dictation

    // Null when the user backs out.
    suspend fun scanQr(): String?

    suspend fun takePhoto(): Attachment?

    suspend fun pickImages(): List<Attachment>

    suspend fun pickFiles(): List<Attachment>
}

interface Prefs {
    operator fun get(key: String): String?

    operator fun set(key: String, value: String?)
}

class Attachment(val name: String, val data: ByteArray, val image: Boolean)

interface Dictation {
    // Loudness, 0 to 1.
    val level: StateFlow<Float>

    val partial: StateFlow<String>

    // False when the device cannot listen, or may not.
    suspend fun start(): Boolean

    suspend fun finish(): String

    fun cancel()
}
