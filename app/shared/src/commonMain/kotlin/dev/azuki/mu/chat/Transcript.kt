package dev.azuki.mu.chat

import androidx.compose.runtime.Immutable
import dev.azuki.mu.api.Block
import dev.azuki.mu.api.BlockKind
import dev.azuki.mu.api.Node
import dev.azuki.mu.api.Sender

// While a loop runs, each block is a row of its own, in order. Once it finishes, all but its final
// response folds into one row in their place.
@Immutable
sealed interface Row {
    val at: Int
    val key: Any
        get() = at

    @Immutable data class User(override val at: Int, val text: String) : Row

    @Immutable
    data class AgentMessage(override val at: Int, val text: String, val from: Sender?) : Row

    @Immutable data class Agent(override val at: Int, val text: String) : Row

    @Immutable data class Thought(override val at: Int, val block: Block) : Row

    @Immutable data class Call(override val at: Int, val block: Block) : Row

    @Immutable
    data class Work(
        override val at: Int,
        val steps: List<Block>,
        val started: Long?,
        val finished: Long?,
    ) : Row {
        override val key
            get() = "work.$at"
    }

    @Immutable
    data class Working(override val at: Int, val started: Long?) : Row {
        override val key
            get() = "working"
    }

    @Immutable data class Shell(override val at: Int, val block: Block) : Row

    @Immutable data class Notice(override val at: Int, val block: Block) : Row
}

@Immutable data class PathNode(val id: Long, val siblings: List<Long>, val blocks: IntRange)

class Transcript {
    private val blocks = mutableListOf<Block>()
    private val nodes = mutableListOf<PathNode>()
    private val settled = mutableListOf<Row>()
    private var last = 0

    val size
        get() = blocks.size

    val path: List<PathNode>
        get() = nodes.toList()

    fun reset(keep: Int) {
        nodes.removeAll { it.blocks.first >= keep }
        if (keep >= blocks.size) return
        blocks.subList(keep, blocks.size).clear()
        settled.clear()
        last = 0
        settle(0)
    }

    fun commit(committed: List<Node>) {
        val from = blocks.size
        for (node in committed) {
            val start = blocks.size
            blocks += node.blocks
            nodes += PathNode(node.id, node.siblings, start until blocks.size)
        }
        settle(from)
    }

    fun rows(live: List<Block>, running: Boolean): List<Row> {
        val joined =
            object : AbstractList<Block>() {
                override val size = blocks.size + live.size

                override fun get(index: Int) =
                    if (index < blocks.size) blocks[index] else live[index - blocks.size]
            }
        val all = ArrayList<Row>(settled.size + 8)
        all += settled
        turn(joined, last, joined.size, running, all)
        return all
    }

    private fun settle(from: Int) {
        for (i in from until blocks.size) {
            if (blocks[i].kind.opens && i > last) {
                turn(blocks, last, i, false, settled)
                last = i
            }
        }
    }
}

private val BlockKind.opens
    get() = this == BlockKind.User || this == BlockKind.AgentMessage

private val BlockKind.notice
    get() = this == BlockKind.Notice || this == BlockKind.Warning

private fun turn(
    blocks: List<Block>,
    start: Int,
    end: Int,
    running: Boolean,
    into: MutableList<Row>,
) {
    var i = start
    val opening = blocks.getOrNull(start)?.takeIf { start < end && it.kind.opens }
    if (opening != null) {
        into +=
            if (opening.kind == BlockKind.User) Row.User(i, opening.text)
            else Row.AgentMessage(i, opening.text, opening.from)
        i++
    }
    if (running) {
        for (j in i until end) single(blocks[j], j, into)
        into += Row.Working(end, opening?.span?.started)
        return
    }
    while (i < end && blocks[i].kind.notice) {
        into += Row.Notice(i, blocks[i])
        i++
    }
    val rest = blocks.subList(i, end)
    val folded = rest.indexOfLast { it.kind == BlockKind.Thought || it.kind == BlockKind.Call } + 1
    if (folded > 0) {
        val steps = rest.subList(0, folded).toList()
        val spans = steps.mapNotNull { it.span }
        into +=
            Row.Work(
                at = i,
                steps = steps,
                started = spans.minOfOrNull { it.started } ?: opening?.span?.started,
                finished = spans.maxOfOrNull { it.finished },
            )
    }
    for (j in i + folded until end) single(blocks[j], j, into)
}

private fun single(block: Block, at: Int, into: MutableList<Row>) {
    when (block.kind) {
        BlockKind.Agent -> if (block.text.isNotBlank()) into += Row.Agent(at, block.text)
        BlockKind.Thought -> if (block.text.isNotBlank()) into += Row.Thought(at, block)
        BlockKind.Call -> into += Row.Call(at, block)
        BlockKind.Shell -> into += Row.Shell(at, block)
        BlockKind.Notice,
        BlockKind.Warning -> into += Row.Notice(at, block)
        BlockKind.User,
        BlockKind.AgentMessage -> Unit
    }
}
