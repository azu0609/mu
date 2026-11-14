package dev.azuki.mu.android

import android.app.Application
import dev.azuki.mu.core.Core

class MuApplication : Application() {
    // One for the whole process: the paired hosts and the connections to them.
    // Their tokens live in the app's private files.
    val core: Core by lazy { Core(filesDir.path) }
}
