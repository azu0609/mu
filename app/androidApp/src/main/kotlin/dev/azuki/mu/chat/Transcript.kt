package dev.azuki.mu.chat

import androidx.compose.runtime.Immutable
import dev.azuki.mu.wire.Block
import dev.azuki.mu.wire.Kind
import dev.azuki.mu.wire.Node
import dev.azuki.mu.wire.Sender

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
        val started: ULong?,
        val finished: ULong?,
    ) : Row {
        override val key
            get() = "work.$at"
    }

    @Immutable
    data class Working(override val at: Int, val started: ULong?) : Row {
        override val key
            get() = "working"
    }

    @Immutable data class Shell(override val at: Int, val block: Block) : Row

    @Immutable data class Notice(override val at: Int, val block: Block) : Row

    @Immutable
    data class Ended(override val at: Int, val node: ULong, val finished: ULong) : Row {
        override val key
            get() = "ended.$at"
    }
}

@Immutable
data class PathNode(
    val id: ULong,
    val siblings: List<ULong>,
    val blocks: IntRange,
    val finished: ULong?,
)

class Transcript {
    private val blocks = mutableListOf<Block>()
    private val nodes = mutableListOf<PathNode>()

    private val endings = mutableMapOf<Int, PathNode>()
    private val settled = mutableListOf<Row>()
    private var last = 0

    val size
        get() = blocks.size

    val path: List<PathNode>
        get() = nodes.toList()

    fun reset(keep: Int) {
        nodes.removeAll { it.blocks.first >= keep }
        endings.keys.removeAll { it >= keep }
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
            val added = PathNode(node.id, node.siblings, start until blocks.size, node.finished)
            nodes += added
            if (added.finished != null && !added.blocks.isEmpty())
                endings[added.blocks.last] = added
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
        turn(joined, last, joined.size, running, all, endings)
        return all
    }

    private fun settle(from: Int) {
        for (i in from until blocks.size) {
            if (blocks[i].kind.opens && i > last) {
                turn(blocks, last, i, false, settled, endings)
                last = i
            }
        }
    }
}

private val Kind.opens
    get() = this == Kind.USER || this == Kind.AGENT_MESSAGE

private val Kind.notice
    get() = this == Kind.NOTICE || this == Kind.WARNING

private fun turn(
    blocks: List<Block>,
    start: Int,
    end: Int,
    running: Boolean,
    into: MutableList<Row>,
    endings: Map<Int, PathNode>,
) {
    var i = start
    val opening = blocks.getOrNull(start)?.takeIf { start < end && it.kind.opens }
    if (opening != null) {
        into +=
            if (opening.kind == Kind.USER) Row.User(i, opening.text)
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
    val folded = rest.indexOfLast { it.kind == Kind.THOUGHT || it.kind == Kind.CALL } + 1
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
    val ending = (end - 1 downTo start).firstNotNullOfOrNull { endings[it] } ?: return
    into += Row.Ended(ending.blocks.last, ending.id, ending.finished ?: return)
}

private fun single(block: Block, at: Int, into: MutableList<Row>) {
    when (block.kind) {
        Kind.AGENT -> if (block.text.isNotBlank()) into += Row.Agent(at, block.text)
        Kind.THOUGHT -> if (block.text.isNotBlank()) into += Row.Thought(at, block)
        Kind.CALL -> into += Row.Call(at, block)
        Kind.SHELL -> into += Row.Shell(at, block)
        Kind.NOTICE,
        Kind.WARNING -> into += Row.Notice(at, block)

        Kind.USER,
        Kind.AGENT_MESSAGE -> Unit
    }
}
