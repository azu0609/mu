package dev.azuki.mu.api

data class Host(
    val id: String,
    val name: String,
    val icon: String?,
    val description: String,
    val address: String,
)

data class HostProfile(
    val name: String,
    val home: String,
    val icon: String?,
    val description: String,
    val snippets: List<Snippet>,
)

data class Snippet(val id: String, val name: String, val command: String, val icon: String?)

enum class Status {
    Idle,
    Running,
    Finished,
    Failed,
}

data class SessionSummary(
    val id: String,
    val title: String,
    val cwd: String,
    // The session that spawned it, if any.
    val parent: String?,
    val status: Status,
    val error: String?,
    val unread: Boolean,
    // Milliseconds since the Unix epoch.
    val updated: Long,
)

sealed interface HostUpdate {
    data class Host(val profile: HostProfile, val sessions: List<SessionSummary>) : HostUpdate

    data class Lost(val reason: String, val unpaired: Boolean) : HostUpdate
}

data class Deleted(val deleted: Long, val inUse: Long)

data class Directory(val path: String, val entries: List<DirectoryEntry>)

data class DirectoryEntry(val name: String, val kind: EntryKind)

enum class EntryKind {
    File,
    Directory,
    Repository,
}

data class Skill(val name: String, val description: String)

// `start` and `end` bound the mention being typed, in characters rather than UTF-16 units.
data class Mention(val start: Int, val end: Int, val query: String, val entries: List<MentionEntry>)

// `text` replaces the mention; a directory's leaves it open for the next segment.
data class MentionEntry(val text: String, val label: String)

// A mention as written, `path` unescaped, in characters rather than UTF-16 units.
data class Written(val start: Int, val end: Int, val path: String, val closed: Boolean)

// What a mention, by its path as written, attaches: `file` is where the host found it.
data class Resolved(val path: String, val file: String, val kind: Attached)

sealed interface Attached {
    data object Image : Attached

    data class Text(val lines: Int, val head: String) : Attached
}

data class ModelInfo(val name: String, val context: Long?)

// A null effort is the endpoint's own default; the host fills in a null context when it can.
data class Model(val name: String, val effort: String?, val context: Long? = null)

enum class BlockKind {
    User,

    // From another session, which the block's `from` names.
    AgentMessage,
    Agent,
    Thought,
    Call,
    Shell,
    Notice,
    Warning,
}

data class Block(
    val kind: BlockKind,
    val text: String,
    val tool: Tool?,
    val span: Span?,
    val from: Sender? = null,
)

// A session as it was when it sent a message.
data class Sender(val session: String, val title: String)

// Milliseconds since the Unix epoch.
data class Span(val started: Long, val finished: Long)

data class Tool(val status: ToolStatus, val output: String, val truncated: Boolean)

sealed interface ToolStatus {
    data object Pending : ToolStatus

    data object Running : ToolStatus

    data class Exited(val code: Int) : ToolStatus

    data class TimedOut(val millis: Long) : ToolStatus

    data object Cancelled : ToolStatus

    data object Failed : ToolStatus
}

data class SessionState(
    val status: Status,
    val title: String,
    val model: Model,
    val contextUsed: Long,
    val input: Long,
    val uncachedInput: Long,
    val output: Long,
    val queued: List<String>,
    val unreadFrom: Long,
)

// `siblings` includes itself, oldest first. `finished` is when it ended a turn, in milliseconds
// since the Unix epoch.
data class Node(
    val id: Long,
    val siblings: List<Long>,
    val blocks: List<Block>,
    val finished: Long?,
)

data class TreeEntry(
    val node: Long,
    val parent: Long?,
    val kind: BlockKind?,
    val text: String,
    val current: Boolean,
)

data class Resume(val epoch: Long, val blocks: Int)

// A reset clears everything but the first `keep` blocks of the transcript; the
// rest of it then arrives as ordinary commits.
sealed interface SessionUpdate {
    data class Reset(
        val epoch: Long,
        val keep: Int,
        val cwd: String,
        val state: SessionState,
        val skills: List<Skill>,
    ) : SessionUpdate

    data class State(val state: SessionState, val started: Boolean) : SessionUpdate

    data class Committed(val nodes: List<Node>) : SessionUpdate

    data class Live(val index: Int, val block: Block) : SessionUpdate

    data class LiveNotice(val block: Block) : SessionUpdate

    data class Feedback(val block: Block) : SessionUpdate

    data class Lost(val reason: String) : SessionUpdate
}

sealed interface ShellUpdate {
    data class Output(val tool: Tool) : ShellUpdate

    data class Done(val run: String, val tool: Tool) : ShellUpdate

    data class Failed(val message: String) : ShellUpdate
}

class MuException(message: String, val unpaired: Boolean = false) : Exception(message)
