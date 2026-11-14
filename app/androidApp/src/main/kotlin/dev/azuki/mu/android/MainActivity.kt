package dev.azuki.mu.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.azuki.mu.App
import dev.azuki.mu.core.version

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val core = (application as MuApplication).core
        setContent {
            App(version = version(), hosts = core.hosts().map { it.name })
        }
    }
}
