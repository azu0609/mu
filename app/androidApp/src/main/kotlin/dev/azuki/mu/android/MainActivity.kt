package dev.azuki.mu.android

import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.azuki.mu.App

class MainActivity : ComponentActivity() {
    private var invite by mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val core = FfiCore((application as MuApplication).core, deviceName())
        val platform = AndroidPlatform(this)
        if (savedInstanceState == null) invite = intent.invite()
        setContent { App(core, platform, invite, onInviteHandled = { invite = null }) }
    }

    // Ahead of anything resuming on a connection that died in the background.
    override fun onStart() {
        (application as MuApplication).core.nudge()
        super.onStart()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        intent.invite()?.let { invite = it }
    }

    private fun Intent.invite(): String? = data?.takeIf { it.scheme == "mu" }?.toString()

    // What the host lists this device as.
    private fun deviceName(): String =
        Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)
            ?.takeIf(String::isNotBlank) ?: "${Build.MANUFACTURER} ${Build.MODEL}"
}
