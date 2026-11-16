package dev.azuki.mu

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
