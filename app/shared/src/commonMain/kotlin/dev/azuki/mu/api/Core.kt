package dev.azuki.mu.api

import kotlinx.coroutines.flow.Flow

// Everything that talks to hosts. Calls throw MuException.
interface Core {
    val version: String

    fun compact(n: Long): String

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

    // Null when the cursor is not in a mention. `refresh` rebuilds the host's file index.
    suspend fun completeMention(
        host: String,
        cwd: String,
        text: String,
        cursor: Int,
        refresh: Boolean,
    ): Mention?

    fun mentions(text: String): List<Written>

    // Only the mentions that sending `text` would attach, as things are now.
    suspend fun resolveMentions(
        host: String,
        cwd: String,
        text: String,
        skill: String?,
    ): List<Resolved>

    suspend fun readImage(host: String, path: String): ByteArray

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

    // False when it was sent before it could be taken back.
    suspend fun recallQueued(index: Int, text: String): Boolean
}
