package dev.azuki.mu.android

import dev.azuki.mu.core.Core as Ffi
import dev.azuki.mu.core.HostListener
import dev.azuki.mu.core.MuException as FfiException
import dev.azuki.mu.core.SessionListener
import dev.azuki.mu.core.ShellListener
import dev.azuki.mu.model.Block
import dev.azuki.mu.model.BlockKind
import dev.azuki.mu.model.Core
import dev.azuki.mu.model.Deleted
import dev.azuki.mu.model.Directory
import dev.azuki.mu.model.DirectoryEntry
import dev.azuki.mu.model.EntryKind
import dev.azuki.mu.model.Host
import dev.azuki.mu.model.HostProfile
import dev.azuki.mu.model.HostUpdate
import dev.azuki.mu.model.Mention
import dev.azuki.mu.model.MentionEntry
import dev.azuki.mu.model.ModelInfo
import dev.azuki.mu.model.MuException
import dev.azuki.mu.model.Session
import dev.azuki.mu.model.SessionState
import dev.azuki.mu.model.SessionSummary
import dev.azuki.mu.model.SessionUpdate
import dev.azuki.mu.model.ShellUpdate
import dev.azuki.mu.model.Skill
import dev.azuki.mu.model.Snippet
import dev.azuki.mu.model.Span
import dev.azuki.mu.model.Status
import dev.azuki.mu.model.Tool
import dev.azuki.mu.model.ToolStatus
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

    override fun hosts(): List<Host> = core.hosts().map { it.toModel() }

    override suspend fun pair(invite: String): Host = mapped { core.pair(invite, device).toModel() }

    override fun forget(host: String) = mapped { core.forget(host) }

    override fun watch(host: String): Flow<HostUpdate> =
        callbackFlow {
                val handle = mapped {
                    core.watchHost(
                        host,
                        object : HostListener {
                            override fun onUpdate(update: dev.azuki.mu.core.HostUpdate) {
                                trySend(update.toModel())
                            }
                        },
                    )
                }
                awaitClose { handle.close() }
            }
            .buffer(Channel.CONFLATED)

    override suspend fun createSession(
        host: String,
        cwd: String,
        model: String,
        effort: String?,
        text: String,
        skill: String?,
    ): String = mapped { core.createSession(host, cwd, model, effort, text, skill) }

    override suspend fun listModels(host: String): List<ModelInfo> = mapped {
        core.listModels(host).map { ModelInfo(it.name, it.context) }
    }

    override suspend fun deleteSessions(host: String): Deleted = mapped {
        core.deleteSessions(host).let { Deleted(it.deleted, it.inUse) }
    }

    override suspend fun setIcon(host: String, icon: String?) = mapped { core.setIcon(host, icon) }

    override suspend fun setDescription(host: String, description: String) = mapped {
        core.setDescription(host, description)
    }

    override suspend fun saveSnippet(
        host: String,
        id: String?,
        name: String,
        command: String,
        icon: String?,
    ): String = mapped { core.saveSnippet(host, id, name, command, icon) }

    override suspend fun deleteSnippet(host: String, id: String) = mapped {
        core.deleteSnippet(host, id)
    }

    override suspend fun listDirectory(host: String, path: String): Directory = mapped {
        val directory = core.listDirectory(host, path)
        Directory(
            directory.path,
            directory.entries.map { entry ->
                DirectoryEntry(
                    entry.name,
                    when (entry.kind) {
                        dev.azuki.mu.core.EntryKind.FILE -> EntryKind.File
                        dev.azuki.mu.core.EntryKind.DIRECTORY -> EntryKind.Directory
                        dev.azuki.mu.core.EntryKind.REPOSITORY -> EntryKind.Repository
                    },
                )
            },
        )
    }

    override suspend fun listSkills(host: String, cwd: String): List<Skill> = mapped {
        core.listSkills(host, cwd).map { it.toModel() }
    }

    override suspend fun completeMention(
        host: String,
        cwd: String,
        text: String,
        cursor: Int,
        refresh: Boolean,
    ): Mention? = mapped {
        core.completeMention(host, cwd, text, cursor.toUInt(), refresh)?.let { mention ->
            Mention(
                mention.start.toInt(),
                mention.end.toInt(),
                mention.query,
                mention.entries.map { MentionEntry(it.text, it.label) },
            )
        }
    }

    override suspend fun upload(host: String, name: String, data: ByteArray): String = mapped {
        core.upload(host, name, data)
    }

    // Each update replaces the last, so only the latest matters.
    override fun shell(host: String, cwd: String, command: String): Flow<ShellUpdate> =
        callbackFlow {
                val handle = mapped {
                    core.shell(
                        host,
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

    override suspend fun openSession(host: String, session: String): Session {
        // None may be dropped: each update builds on the last.
        val updates = Channel<SessionUpdate>(Channel.UNLIMITED)
        val handle = mapped {
            core.openSession(
                host,
                session,
                object : SessionListener {
                    override fun onUpdate(update: dev.azuki.mu.core.SessionUpdate) {
                        updates.trySend(update.toModel())
                    }
                },
            )
        }
        return object : Session {
            override val updates: Flow<SessionUpdate> = updates.receiveAsFlow()

            override suspend fun send(text: String, skill: String?) = mapped {
                handle.send(text, skill)
            }

            override suspend fun appendShell(run: String) = mapped { handle.appendShell(run) }

            override suspend fun setModel(name: String, effort: String?) = mapped {
                handle.setModel(name, effort)
            }

            override suspend fun stop() = mapped { handle.stop() }

            override suspend fun markRead() = mapped { handle.markRead() }

            override fun close() {
                handle.close()
                updates.close()
            }
        }
    }
}

private inline fun <T> mapped(call: () -> T): T =
    try {
        call()
    } catch (e: FfiException) {
        throw MuException(e.message.orEmpty(), unpaired = e is FfiException.Unpaired)
    }

private fun dev.azuki.mu.core.Host.toModel() = Host(id, name, icon, description, address)

private fun dev.azuki.mu.core.Skill.toModel() = Skill(name, description)

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
                sessions.map { it.toModel() },
            )
        dev.azuki.mu.core.HostUpdate.Lost -> HostUpdate.Lost
    }

private fun dev.azuki.mu.core.SessionSummary.toModel() =
    SessionSummary(id, title, cwd, status.toModel(), error, unread, updated)

private fun dev.azuki.mu.core.Status.toModel() =
    when (this) {
        dev.azuki.mu.core.Status.IDLE -> Status.Idle
        dev.azuki.mu.core.Status.RUNNING -> Status.Running
        dev.azuki.mu.core.Status.FINISHED -> Status.Finished
        dev.azuki.mu.core.Status.FAILED -> Status.Failed
    }

private fun dev.azuki.mu.core.SessionState.toModel() =
    SessionState(
        status.toModel(),
        title,
        model,
        effort,
        context,
        contextUsed,
        input,
        uncachedInput,
        output,
        queued,
    )

private fun dev.azuki.mu.core.SessionUpdate.toModel(): SessionUpdate =
    when (this) {
        is dev.azuki.mu.core.SessionUpdate.Reset ->
            SessionUpdate.Reset(keep, cwd, state.toModel(), skills.map { it.toModel() })
        is dev.azuki.mu.core.SessionUpdate.State -> SessionUpdate.State(state.toModel(), started)
        is dev.azuki.mu.core.SessionUpdate.Committed ->
            SessionUpdate.Committed(blocks.map { it.toModel() })
        is dev.azuki.mu.core.SessionUpdate.Live -> SessionUpdate.Live(index, block.toModel())
        is dev.azuki.mu.core.SessionUpdate.LiveNotice -> SessionUpdate.LiveNotice(block.toModel())
        is dev.azuki.mu.core.SessionUpdate.Feedback -> SessionUpdate.Feedback(block.toModel())
        dev.azuki.mu.core.SessionUpdate.Lost -> SessionUpdate.Lost
    }

private fun dev.azuki.mu.core.ShellUpdate.toModel(): ShellUpdate =
    when (this) {
        is dev.azuki.mu.core.ShellUpdate.Output -> ShellUpdate.Output(tool.toModel())
        is dev.azuki.mu.core.ShellUpdate.Done -> ShellUpdate.Done(run, tool.toModel())
        is dev.azuki.mu.core.ShellUpdate.Failed -> ShellUpdate.Failed(message)
    }

private fun dev.azuki.mu.core.Block.toModel() =
    Block(
        when (kind) {
            dev.azuki.mu.core.BlockKind.USER -> BlockKind.User
            dev.azuki.mu.core.BlockKind.AGENT -> BlockKind.Agent
            dev.azuki.mu.core.BlockKind.THOUGHT -> BlockKind.Thought
            dev.azuki.mu.core.BlockKind.CALL -> BlockKind.Call
            dev.azuki.mu.core.BlockKind.SHELL -> BlockKind.Shell
            dev.azuki.mu.core.BlockKind.NOTICE -> BlockKind.Notice
            dev.azuki.mu.core.BlockKind.WARNING -> BlockKind.Warning
        },
        text,
        tool?.toModel(),
        span?.let { Span(it.started, it.finished) },
    )

private fun dev.azuki.mu.core.Tool.toModel() =
    Tool(
        when (val status = status) {
            dev.azuki.mu.core.ToolStatus.Pending -> ToolStatus.Pending
            dev.azuki.mu.core.ToolStatus.Running -> ToolStatus.Running
            is dev.azuki.mu.core.ToolStatus.Exited -> ToolStatus.Exited(status.code)
            is dev.azuki.mu.core.ToolStatus.TimedOut -> ToolStatus.TimedOut(status.millis)
            dev.azuki.mu.core.ToolStatus.Cancelled -> ToolStatus.Cancelled
            dev.azuki.mu.core.ToolStatus.Failed -> ToolStatus.Failed
        },
        output,
        truncated,
    )
