package dev.azuki.mu

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.azuki.mu.platform.Platform

class MainActivity : ComponentActivity() {
    private var invite by mutableStateOf<String?>(null)

    private val core
        get() = (application as MuApplication).core

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val platform = Platform(this)
        if (savedInstanceState == null) invite = intent.invite()
        setContent { App(core, platform, invite, onInviteHandled = { invite = null }) }
    }

    // Ahead of anything resuming on a connection that died in the background.
    override fun onStart() {
        core.nudge()
        super.onStart()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        intent.invite()?.let { invite = it }
    }

    private fun Intent.invite(): String? = data?.takeIf { it.scheme == "mu" }?.toString()
}
