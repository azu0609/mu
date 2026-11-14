package dev.azuki.mu.android

import android.app.Application
import dev.azuki.mu.core.Core

class MuApplication : Application() {
    val core: Core by lazy { Core(filesDir.path) }
}
