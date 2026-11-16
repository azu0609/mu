package dev.azuki.mu.chat

import dev.azuki.mu.api.Block
import dev.azuki.mu.api.BlockKind

sealed interface Entry {
    val key: String

    data class User(override val key: String, val text: String) : Entry

    // All a turn did before its final response, folded behind "Worked for…".
    data class Work(
        override val key: String,
        val steps: List<Step>,
        val started: Long?,
        val finished: Long?,
        val ongoing: Boolean,
    ) : Entry

    data class Agent(override val key: String, val text: String) : Entry

    data class Shell(override val key: String, val block: Block) : Entry

    data class Notice(override val key: String, val block: Block) : Entry
}

data class Step(val key: String, val block: Block)

// Turns the flat transcript into what the chat shows, in the TUI's order: what is committed, the
// live notice, then what is live. `live` blocks are keyed by the index they get once committed,
// so the list keeps their items across commits.
fun entries(blocks: List<Block>, notice: Block?, live: List<Block>, running: Boolean): List<Entry> {
    val all =
        blocks.mapIndexed { i, block -> Step("c$i", block) } +
            listOfNotNull(notice?.let { Step("notice", it) }) +
            live.mapIndexed { i, block -> Step("c${blocks.size + i}", block) }
    val entries = mutableListOf<Entry>()
    var turn = mutableListOf<Step>()
    var turnKey = "start"
    var sent: Long? = null

    fun show(step: Step) {
        val block = step.block
        when (block.kind) {
            BlockKind.Agent ->
                if (block.text.isNotBlank()) entries += Entry.Agent(step.key, block.text)
            BlockKind.Shell -> entries += Entry.Shell(step.key, block)
            BlockKind.Notice,
            BlockKind.Warning -> entries += Entry.Notice(step.key, block)
            BlockKind.User,
            BlockKind.Thought,
            BlockKind.Call -> Unit
        }
    }

    fun flush(last: Boolean) {
        val ongoing = last && running
        // What came with the message, like its attachments, stays out of the fold.
        val lead = turn.takeWhile {
            it.block.kind == BlockKind.Notice || it.block.kind == BlockKind.Warning
        }
        val rest = turn.drop(lead.size)
        val folded =
            rest.indexOfLast {
                it.block.kind == BlockKind.Thought || it.block.kind == BlockKind.Call
            } + 1
        lead.forEach(::show)
        if (folded > 0 || ongoing) {
            val spans = turn.mapNotNull { it.block.span }
            entries +=
                Entry.Work(
                    key = "$turnKey.work",
                    steps = rest.take(folded),
                    started = spans.minOfOrNull { it.started } ?: sent,
                    finished = spans.maxOfOrNull { it.finished },
                    ongoing = ongoing,
                )
        }
        rest.drop(folded).forEach(::show)
        turn = mutableListOf()
    }

    for (step in all) {
        if (step.block.kind == BlockKind.User) {
            flush(last = false)
            entries += Entry.User(step.key, step.block.text)
            turnKey = step.key
            sent = step.block.span?.started
        } else {
            turn += step
        }
    }
    flush(last = true)
    return entries
}
