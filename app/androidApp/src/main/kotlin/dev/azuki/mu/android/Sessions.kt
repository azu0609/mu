package dev.azuki.mu.android

import dev.azuki.mu.core.Core
import dev.azuki.mu.core.SessionListListener
import dev.azuki.mu.core.SessionListUpdate
import dev.azuki.mu.core.SessionListener
import dev.azuki.mu.core.SessionUpdate
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
        val handle = openSession(
            host,
            session,
            object : SessionListener {
                override fun onUpdate(update: SessionUpdate) {
                    trySend(update)
                }
            },
        )
        awaitClose { handle.close() }
    }.buffer(Channel.UNLIMITED)

// A host's session list. Each update is the whole list, so only the latest matters.
fun Core.sessionList(host: String): Flow<SessionListUpdate> =
    callbackFlow {
        val handle = watchSessions(
            host,
            object : SessionListListener {
                override fun onUpdate(update: SessionListUpdate) {
                    trySend(update)
                }
            },
        )
        awaitClose { handle.close() }
    }.buffer(Channel.CONFLATED)
