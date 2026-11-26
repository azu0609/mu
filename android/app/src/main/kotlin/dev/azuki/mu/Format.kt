package dev.azuki.mu

import dev.azuki.mu.wire.ToolStatus

fun duration(millis: Long): String {
    val seconds = (millis / 1000).coerceAtLeast(0)
    return when {
        seconds < 60 -> "${seconds}s"
        seconds < 3600 -> "${seconds / 60}m"
        else -> "${seconds / 3600}h ${seconds % 3600 / 60}m"
    }
}

// `~/…` for paths under the host's home.
fun tilde(path: String, home: String?): String =
    when {
        home.isNullOrEmpty() -> path
        path == home -> "~"
        path.startsWith("$home/") -> "~" + path.removePrefix(home)
        else -> path
    }

fun parentOf(path: String): String = path.trimEnd('/').substringBeforeLast('/', "").ifEmpty { "/" }

fun baseName(path: String): String = path.trimEnd('/').substringAfterLast('/').ifEmpty { "/" }

// How a command ended, unless it just succeeded or is yet to end.
fun ending(status: ToolStatus): String? =
    when (status) {
        is ToolStatus.Exited -> if (status.v1 == 0) null else "exit ${status.v1}"
        is ToolStatus.TimedOut -> "timed out"
        ToolStatus.Cancelled -> "cancelled"
        ToolStatus.Error -> "failed"
        ToolStatus.Running,
        ToolStatus.Pending -> null
    }
