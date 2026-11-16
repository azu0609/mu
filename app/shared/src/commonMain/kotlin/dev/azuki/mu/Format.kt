package dev.azuki.mu

private val UNITS = listOf("k" to 1_000L, "M" to 1_000_000L, "B" to 1_000_000_000L)

fun compact(n: Long): String {
    if (n < 1_000) return n.toString()
    var (suffix, multiplier) = UNITS[0]
    for ((nextSuffix, next) in UNITS.drop(1)) {
        if (n < next - multiplier / 20) break
        suffix = nextSuffix
        multiplier = next
    }
    val tenths = (n * 10 + multiplier / 2) / multiplier
    return "${tenths / 10}.${tenths % 10}$suffix"
}

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
