package dev.azuki.mu.design

import androidx.compose.ui.graphics.vector.ImageVector

// The composer's own send and return keys, and Send's mirror for taking a message back, drawn to
// sit with Solar's strokes.
object Glyphs {
    val Send: ImageVector by lazy {
        icon("Send") {
            stroke("M12 19.5V5", width = 2f)
            stroke("M5.5 11.5L12 5L18.5 11.5", width = 2f, roundJoin = true)
        }
    }

    val Recall: ImageVector by lazy {
        icon("Recall") {
            stroke("M12 4.5V19", width = 2f)
            stroke("M5.5 12.5L12 19L18.5 12.5", width = 2f, roundJoin = true)
        }
    }

    val Return: ImageVector by lazy {
        icon("Return") {
            stroke("M19 5.5V10.5C19 12.157 17.657 13.5 16 13.5H5.5", width = 2f, roundJoin = true)
            stroke("M9.5 9.5L5.5 13.5L9.5 17.5", width = 2f, roundJoin = true)
        }
    }

    val Stop: ImageVector by lazy {
        icon("Stop") {
            fill(
                "M8.5 7H15.5C16.328 7 17 7.672 17 8.5V15.5C17 16.328 16.328 17 15.5 17H8.5C7.672 17 7 16.328 7 15.5V8.5C7 7.672 7.672 7 8.5 7Z"
            )
        }
    }
}
