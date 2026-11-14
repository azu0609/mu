package dev.azuki.mu.android

import dev.azuki.mu.core.Core
import dev.azuki.mu.core.HostListener
import dev.azuki.mu.core.HostUpdate
import dev.azuki.mu.core.SessionListener
import dev.azuki.mu.core.SessionUpdate
import dev.azuki.mu.core.ShellListener
import dev.azuki.mu.core.ShellUpdate
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow

// The core calls listeners on its own threads, so these only hand updates over,
// in order, to whatever dispatcher collects them.

// A session's updates. None may be dropped: each one builds on the last.
fun Core.updates(host: String, session: String): Flow<SessionUpdate> =
    callbackFlow {
            val handle =
                openSession(
                    host,
                    session,
                    object : SessionListener {
                        override fun onUpdate(update: SessionUpdate) {
                            trySend(update)
                        }
                    },
                )
            awaitClose { handle.close() }
        }
        .buffer(Channel.UNLIMITED)

// A host's profile and session list. Each update is all of it, so only the latest matters.
fun Core.hostUpdates(host: String): Flow<HostUpdate> =
    callbackFlow {
            val handle =
                watchHost(
                    host,
                    object : HostListener {
                        override fun onUpdate(update: HostUpdate) {
                            trySend(update)
                        }
                    },
                )
            awaitClose { handle.close() }
        }
        .buffer(Channel.CONFLATED)

// A command run in shell mode. The flow completes after its final update, Done or
// Failed; cancelling it before then kills the command. Each update replaces the last,
// so only the latest matters.
fun Core.shellUpdates(host: String, cwd: String, command: String): Flow<ShellUpdate> =
    callbackFlow {
            val handle =
                shell(
                    host,
                    cwd,
                    command,
                    object : ShellListener {
                        override fun onUpdate(update: ShellUpdate) {
                            trySend(update)
                            if (update !is ShellUpdate.Output) close()
                        }
                    },
                )
            awaitClose { handle.close() }
        }
        .buffer(Channel.CONFLATED)
