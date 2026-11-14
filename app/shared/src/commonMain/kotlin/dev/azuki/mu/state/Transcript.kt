package dev.azuki.mu.state

import dev.azuki.mu.model.Block
import dev.azuki.mu.model.BlockKind

sealed interface Entry {
    val key: String

    data class User(override val key: String, val text: String) : Entry

    // Thoughts and calls of one turn, folded behind "Worked for…".
    data class Work(
        override val key: String,
        val blocks: List<Block>,
        val started: Long?,
        val finished: Long?,
        val ongoing: Boolean,
    ) : Entry

    data class Agent(override val key: String, val text: String) : Entry

    data class Shell(override val key: String, val block: Block) : Entry

    data class Notice(override val key: String, val block: Block) : Entry
}

// Turns the flat transcript into what the chat shows. `live` blocks belong to the last turn and
// are keyed by the index they get once committed, so the list keeps their items across commits.
fun entries(blocks: List<Block>, live: List<Block>, running: Boolean): List<Entry> {
    val all = (blocks + live).mapIndexed { i, block -> "c$i" to block }
    val entries = mutableListOf<Entry>()
    var turn = mutableListOf<Pair<String, Block>>()
    var turnKey = "start"

    fun flush(last: Boolean) {
        if (turn.isEmpty() && !(last && running)) return
        val work = turn.filter { (_, block) ->
            block.kind == BlockKind.Thought || block.kind == BlockKind.Call
        }
        val ongoing = last && running
        if (work.isNotEmpty() || ongoing) {
            val spans = turn.mapNotNull { it.second.span }
            entries +=
                Entry.Work(
                    key = "$turnKey.work",
                    blocks = work.map { it.second },
                    started = spans.minOfOrNull { it.started },
                    finished = spans.maxOfOrNull { it.finished },
                    ongoing = ongoing,
                )
        }
        for ((key, block) in turn) {
            when (block.kind) {
                BlockKind.Agent ->
                    if (block.text.isNotBlank()) entries += Entry.Agent(key, block.text)
                BlockKind.Shell -> entries += Entry.Shell(key, block)
                BlockKind.Notice,
                BlockKind.Warning -> entries += Entry.Notice(key, block)
                BlockKind.Thought,
                BlockKind.Call,
                BlockKind.User -> Unit
            }
        }
        turn = mutableListOf()
    }

    for ((key, block) in all) {
        if (block.kind == BlockKind.User) {
            flush(last = false)
            entries += Entry.User(key, block.text)
            turnKey = key
        } else {
            turn += key to block
        }
    }
    flush(last = true)
    return entries
}

private val UNITS = listOf("k" to 1_000L, "M" to 1_000_000L, "B" to 1_000_000_000L)

fun compact(n: Long): String {
    if (n < 1_000) return n.toString()
    var (suffix, multiplier) = UNITS[0]
    for ((nextSuffix, next) in UNITS.drop(1)) {
        if (n < next - multiplier / 20) break
        suffix = nextSuffix
        multiplier = next
    }
    val tenths = (n * 10 + multiplier / 2) / multiplier
    return "${tenths / 10}.${tenths % 10}$suffix"
}

fun duration(millis: Long): String {
    val seconds = (millis / 1000).coerceAtLeast(0)
    return when {
        seconds < 60 -> "${seconds}s"
        seconds < 3600 -> "${seconds / 60}m"
        else -> "${seconds / 3600}h ${seconds % 3600 / 60}m"
    }
}

// `~/…` for paths under the host's home.
fun tilde(path: String, home: String?): String =
    when {
        home.isNullOrEmpty() -> path
        path == home -> "~"
        path.startsWith("$home/") -> "~" + path.removePrefix(home)
        else -> path
    }

fun parentOf(path: String): String = path.trimEnd('/').substringBeforeLast('/', "").ifEmpty { "/" }

fun baseName(path: String): String = path.trimEnd('/').substringAfterLast('/').ifEmpty { "/" }
