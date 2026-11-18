package dev.azuki.mu.android

import dev.azuki.mu.api.Attached
import dev.azuki.mu.api.Block
import dev.azuki.mu.api.BlockKind
import dev.azuki.mu.api.Core
import dev.azuki.mu.api.Deleted
import dev.azuki.mu.api.Directory
import dev.azuki.mu.api.DirectoryEntry
import dev.azuki.mu.api.EntryKind
import dev.azuki.mu.api.Host
import dev.azuki.mu.api.HostProfile
import dev.azuki.mu.api.HostUpdate
import dev.azuki.mu.api.Mention
import dev.azuki.mu.api.MentionEntry
import dev.azuki.mu.api.Model
import dev.azuki.mu.api.ModelInfo
import dev.azuki.mu.api.MuException
import dev.azuki.mu.api.Remote
import dev.azuki.mu.api.Resolved
import dev.azuki.mu.api.Resume
import dev.azuki.mu.api.Sender
import dev.azuki.mu.api.Session
import dev.azuki.mu.api.SessionState
import dev.azuki.mu.api.SessionSummary
import dev.azuki.mu.api.SessionUpdate
import dev.azuki.mu.api.ShellUpdate
import dev.azuki.mu.api.Skill
import dev.azuki.mu.api.Snippet
import dev.azuki.mu.api.Span
import dev.azuki.mu.api.Status
import dev.azuki.mu.api.Tool
import dev.azuki.mu.api.ToolStatus
import dev.azuki.mu.api.Written
import dev.azuki.mu.core.Core as Ffi
import dev.azuki.mu.core.HostListener
import dev.azuki.mu.core.MuException as FfiException
import dev.azuki.mu.core.SessionListener
import dev.azuki.mu.core.ShellListener
import dev.azuki.mu.wire.Kind
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.receiveAsFlow

// The Rust core, as the shared UI sees it. Its listeners run on the core's own
// threads; flows and channels carry what they hear over to the UI's.
class FfiCore(private val core: Ffi, private val device: String) : Core {
    override val version: String = dev.azuki.mu.core.version()

    override fun compact(n: Long): String = dev.azuki.mu.core.compact(n.toULong())

    override fun mentions(text: String): List<Written> =
        dev.azuki.mu.core.mentions(text).map {
            Written(it.start.toInt(), it.end.toInt(), it.path, it.closed)
        }

    override fun hosts(): List<Host> = core.hosts().map { it.toModel() }

    override suspend fun pair(invite: String): Host = mapped { core.pair(invite, device).toModel() }

    override fun forget(host: String) = mapped { core.forget(host) }

    override fun remote(host: String): Remote = FfiRemote(core.remote(host))
}

private class FfiRemote(private val remote: dev.azuki.mu.core.Remote) : Remote {
    // Each update is all of it, so only the latest matters.
    override fun watch(): Flow<HostUpdate> =
        callbackFlow {
                val handle = mapped {
                    remote.watch(
                        object : HostListener {
                            override fun onUpdate(update: dev.azuki.mu.core.HostUpdate) {
                                trySend(update.toModel())
                            }
                        }
                    )
                }
                awaitClose { handle.close() }
            }
            .buffer(Channel.CONFLATED)

    override suspend fun createSession(
        cwd: String,
        model: Model,
        text: String,
        skill: String?,
    ): String = mapped { remote.createSession(cwd, model.toFfi(), text, skill) }

    override suspend fun openSession(session: String, resume: Resume?): Session {
        // None may be dropped: each update builds on the last.
        val batches = Channel<List<SessionUpdate>>(Channel.UNLIMITED)
        val handle = mapped {
            remote.openSession(
                session,
                resume?.let { dev.azuki.mu.core.Resume(it.epoch.toULong(), it.blocks.toUInt()) },
                object : SessionListener {
                    override fun onUpdate(updates: List<dev.azuki.mu.core.SessionUpdate>) {
                        batches.trySend(updates.map { it.toModel() })
                    }
                },
            )
        }
        return object : Session {
            override val updates: Flow<List<SessionUpdate>> = batches.receiveAsFlow()

            override suspend fun send(text: String, skill: String?) = mapped {
                handle.send(text, skill)
            }

            override suspend fun appendShell(run: String) = mapped { handle.appendShell(run) }

            override suspend fun setModel(model: Model) = mapped { handle.setModel(model.toFfi()) }

            override suspend fun stop(children: Boolean) = mapped { handle.stop(children) }

            override suspend fun markRead() = mapped { handle.markRead() }

            override suspend fun recallQueued(index: Int, text: String) = mapped {
                handle.recallQueued(index.toUInt(), text)
            }

            override fun close() {
                handle.close()
                batches.close()
            }
        }
    }

    override suspend fun deleteSessions(): Deleted = mapped {
        remote.deleteSessions().let { Deleted(it.sessions.toLong(), it.inUse.toLong()) }
    }

    override suspend fun listModels(): List<ModelInfo> = mapped {
        remote.listModels().map { ModelInfo(it.name, it.context?.toLong()) }
    }

    override suspend fun setIcon(icon: String?) = mapped { remote.setIcon(icon) }

    override suspend fun setDescription(description: String) = mapped {
        remote.setDescription(description)
    }

    override suspend fun saveSnippet(
        id: String?,
        name: String,
        command: String,
        icon: String?,
    ): String = mapped { remote.saveSnippet(id, name, command, icon) }

    override suspend fun deleteSnippet(id: String) = mapped { remote.deleteSnippet(id) }

    override suspend fun listDirectory(path: String): Directory = mapped {
        val directory = remote.listDirectory(path)
        Directory(
            directory.path,
            directory.entries.map { entry ->
                DirectoryEntry(
                    entry.name,
                    when (entry.kind) {
                        dev.azuki.mu.wire.EntryKind.FILE -> EntryKind.File
                        dev.azuki.mu.wire.EntryKind.DIRECTORY -> EntryKind.Directory
                        dev.azuki.mu.wire.EntryKind.REPOSITORY -> EntryKind.Repository
                    },
                )
            },
        )
    }

    override suspend fun listSkills(cwd: String): List<Skill> = mapped {
        remote.listSkills(cwd).map { it.toModel() }
    }

    override suspend fun completeMention(
        cwd: String,
        text: String,
        cursor: Int,
        refresh: Boolean,
    ): Mention? = mapped {
        remote.completeMention(cwd, text, cursor.toUInt(), refresh)?.let { mention ->
            Mention(
                mention.start.toInt(),
                mention.end.toInt(),
                mention.query,
                mention.entries.map { MentionEntry(it.text, it.label) },
            )
        }
    }

    override suspend fun resolveMentions(
        cwd: String,
        text: String,
        skill: String?,
    ): List<Resolved> = mapped {
        remote.resolveMentions(cwd, text, skill).map {
            Resolved(
                it.path,
                it.file,
                when (val kind = it.kind) {
                    dev.azuki.mu.wire.Attached.Image -> Attached.Image
                    is dev.azuki.mu.wire.Attached.Text ->
                        Attached.Text(kind.lines.toInt(), kind.head)
                },
            )
        }
    }

    override suspend fun readImage(path: String): ByteArray = mapped { remote.readImage(path) }

    override suspend fun upload(name: String, data: ByteArray): String = mapped {
        remote.upload(name, data)
    }

    // Each update replaces the last, so only the latest matters.
    override fun shell(cwd: String, command: String): Flow<ShellUpdate> =
        callbackFlow {
                val handle = mapped {
                    remote.shell(
                        cwd,
                        command,
                        object : ShellListener {
                            override fun onUpdate(update: dev.azuki.mu.core.ShellUpdate) {
                                val model = update.toModel()
                                trySend(model)
                                if (model !is ShellUpdate.Output) close()
                            }
                        },
                    )
                }
                awaitClose { handle.close() }
            }
            .buffer(Channel.CONFLATED)
}

private inline fun <T> mapped(call: () -> T): T =
    try {
        call()
    } catch (e: FfiException) {
        throw MuException(e.message.orEmpty(), unpaired = e is FfiException.Unpaired)
    }

private fun Model.toFfi() = dev.azuki.mu.wire.Model(name, effort, context?.toULong())

private fun dev.azuki.mu.core.Host.toModel() = Host(id, name, icon, description, address)

private fun dev.azuki.mu.wire.Skill.toModel() = Skill(name, description)

private fun dev.azuki.mu.core.HostUpdate.toModel(): HostUpdate =
    when (this) {
        is dev.azuki.mu.core.HostUpdate.Host ->
            HostUpdate.Host(
                HostProfile(
                    profile.name,
                    profile.home,
                    profile.icon,
                    profile.description,
                    profile.snippets.map { Snippet(it.id, it.name, it.command, it.icon) },
                ),
                sessions.map {
                    SessionSummary(
                        it.id,
                        it.title,
                        it.cwd,
                        it.parent,
                        it.status.toModel(),
                        it.error,
                        it.unread,
                        it.updated.toLong(),
                    )
                },
            )
        dev.azuki.mu.core.HostUpdate.Lost -> HostUpdate.Lost
    }

private fun dev.azuki.mu.wire.Status.toModel() =
    when (this) {
        dev.azuki.mu.wire.Status.IDLE -> Status.Idle
        dev.azuki.mu.wire.Status.RUNNING -> Status.Running
        dev.azuki.mu.wire.Status.FINISHED -> Status.Finished
        dev.azuki.mu.wire.Status.FAILED -> Status.Failed
    }

private fun dev.azuki.mu.wire.State.toModel() =
    SessionState(
        status.toModel(),
        title,
        Model(model.name, model.effort, model.context?.toLong()),
        stats.contextUsed.toLong(),
        stats.input.toLong(),
        stats.uncachedInput.toLong(),
        stats.output.toLong(),
        queued,
    )

private fun dev.azuki.mu.core.SessionUpdate.toModel(): SessionUpdate =
    when (this) {
        is dev.azuki.mu.core.SessionUpdate.Reset ->
            SessionUpdate.Reset(
                epoch.toLong(),
                keep.toInt(),
                cwd,
                state.toModel(),
                skills.map { it.toModel() },
            )
        is dev.azuki.mu.core.SessionUpdate.State -> SessionUpdate.State(state.toModel(), started)
        is dev.azuki.mu.core.SessionUpdate.Committed ->
            SessionUpdate.Committed(blocks.map { it.toModel() })
        is dev.azuki.mu.core.SessionUpdate.Live ->
            SessionUpdate.Live(index.toInt(), block.toModel())
        is dev.azuki.mu.core.SessionUpdate.LiveNotice -> SessionUpdate.LiveNotice(block.toModel())
        is dev.azuki.mu.core.SessionUpdate.Feedback -> SessionUpdate.Feedback(block.toModel())
        dev.azuki.mu.core.SessionUpdate.Lost -> SessionUpdate.Lost
    }

private fun dev.azuki.mu.core.ShellUpdate.toModel(): ShellUpdate =
    when (this) {
        is dev.azuki.mu.core.ShellUpdate.Output ->
            ShellUpdate.Output(tool(dev.azuki.mu.wire.ToolStatus.Running, output))
        is dev.azuki.mu.core.ShellUpdate.Done -> ShellUpdate.Done(run, tool(status, output))
        is dev.azuki.mu.core.ShellUpdate.Failed -> ShellUpdate.Failed(message)
    }

private fun dev.azuki.mu.wire.Block.toModel() =
    Block(
        when (kind) {
            Kind.USER -> BlockKind.User
            Kind.AGENT_MESSAGE -> BlockKind.AgentMessage
            Kind.AGENT -> BlockKind.Agent
            Kind.THOUGHT -> BlockKind.Thought
            Kind.CALL -> BlockKind.Call
            Kind.SHELL -> BlockKind.Shell
            Kind.NOTICE -> BlockKind.Notice
            Kind.WARNING -> BlockKind.Warning
        },
        text,
        tool?.let { tool(it.status, it.output) },
        span?.let { Span(it.started.toLong(), it.finished.toLong()) },
        from?.let { Sender(it.session, it.title) },
    )

private fun tool(status: dev.azuki.mu.wire.ToolStatus, output: dev.azuki.mu.wire.ToolOutput) =
    Tool(
        when (status) {
            dev.azuki.mu.wire.ToolStatus.Pending -> ToolStatus.Pending
            dev.azuki.mu.wire.ToolStatus.Running -> ToolStatus.Running
            is dev.azuki.mu.wire.ToolStatus.Exited -> ToolStatus.Exited(status.v1)
            is dev.azuki.mu.wire.ToolStatus.TimedOut -> ToolStatus.TimedOut(status.v1.toLong())
            dev.azuki.mu.wire.ToolStatus.Cancelled -> ToolStatus.Cancelled
            dev.azuki.mu.wire.ToolStatus.Error -> ToolStatus.Failed
        },
        output.text,
        output.truncated,
    )
