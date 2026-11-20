package dev.azuki.mu.api

import kotlinx.coroutines.flow.Flow

// Everything that talks to hosts. Calls throw MuException.
interface Core {
    val version: String

    fun compact(n: Long): String

    fun stamp(millis: Long): String

    fun mentions(text: String): List<Written>

    fun hosts(): List<Host>

    suspend fun pair(invite: String): Host

    fun forget(host: String)

    // The paired host with this id, to call; it connects when first called.
    fun remote(host: String): Remote
}

interface Remote {
    // Each update is all of it; the flow keeps going through losses.
    fun watch(): Flow<HostUpdate>

    // Starts the session with its first message.
    suspend fun createSession(cwd: String, model: Model, text: String, skill: String?): String

    fun openSession(session: String, resume: Resume?): Session

    suspend fun deleteSessions(): Deleted

    suspend fun listModels(): List<ModelInfo>

    suspend fun setIcon(icon: String?)

    suspend fun setDescription(description: String)

    suspend fun saveSnippet(id: String?, name: String, command: String, icon: String?): String

    suspend fun deleteSnippet(id: String)

    suspend fun listDirectory(path: String): Directory

    suspend fun listSkills(cwd: String): List<Skill>

    // Null when the cursor is not in a mention. `refresh` rebuilds the host's file index.
    suspend fun completeMention(cwd: String, text: String, cursor: Int, refresh: Boolean): Mention?

    // Only the mentions that sending `text` would attach, as things are now.
    suspend fun resolveMentions(cwd: String, text: String, skill: String?): List<Resolved>

    suspend fun readImage(path: String): ByteArray

    // Where the host keeps the file, for a message to @mention.
    suspend fun upload(name: String, data: ByteArray): String

    fun shell(cwd: String, command: String): Flow<ShellUpdate>
}

interface Session : AutoCloseable {
    // None may be dropped: each update builds on the last.
    val updates: Flow<List<SessionUpdate>>

    suspend fun send(text: String, skill: String?)

    suspend fun appendShell(run: String)

    suspend fun setModel(model: Model)

    // `children` stops the sessions it parents too, and theirs.
    suspend fun stop(children: Boolean)

    suspend fun markRead()

    // False when it was sent before it could be taken back.
    suspend fun recallQueued(index: Int, text: String): Boolean

    suspend fun tree(): List<TreeEntry>

    suspend fun checkout(node: Long?)

    suspend fun edit(node: Long): String

    // To where the conversation last was within the branch `node` starts.
    suspend fun switchBranch(node: Long)
}
