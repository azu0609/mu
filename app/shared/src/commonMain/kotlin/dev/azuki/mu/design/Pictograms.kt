package dev.azuki.mu.design

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.theme.MiuixTheme

// Hosts and snippets name their icon, as "solar:<name>", for every client to draw.
class Pictogram(val name: String, val vector: ImageVector, val tint: Color)

object Pictograms {
    val all =
        listOf(
            Pictogram("solar:server-2-bold-duotone", Solar.Server2BoldDuotone, Color(0xFF7D99A3)),
            Pictogram(
                "solar:box-minimalistic-bold-duotone",
                Solar.BoxMinimalisticBoldDuotone,
                Color(0xFF6E8C93),
            ),
            Pictogram(
                "solar:laptop-minimalistic-bold-duotone",
                Solar.LaptopMinimalisticBoldDuotone,
                Color(0xFF2CCB9C),
            ),
            Pictogram(
                "solar:smartphone-bold-duotone",
                Solar.SmartphoneBoldDuotone,
                Color(0xFFFF9F43),
            ),
            Pictogram("solar:database-bold-duotone", Solar.DatabaseBoldDuotone, Accent),
            Pictogram(
                "solar:star-shine-bold-duotone",
                Solar.StarShineBoldDuotone,
                Color(0xFF9B7BF7),
            ),
            Pictogram(
                "solar:crown-star-bold-duotone",
                Solar.CrownStarBoldDuotone,
                Color(0xFF5AB4F0),
            ),
            Pictogram("solar:cat-bold-duotone", Solar.CatBoldDuotone, Accent),
            Pictogram("solar:git-branch-bold-duotone", Solar.GitBranchBoldDuotone, Accent),
            Pictogram("solar:folder-git-bold-duotone", Solar.FolderGitBoldDuotone, Accent),
            Pictogram("solar:file-stack-bold-duotone", Solar.FileStackBoldDuotone, Accent),
            Pictogram("solar:file-terminal-bold-duotone", Solar.FileTerminalBoldDuotone, Accent),
            Pictogram("solar:file-text-bold-duotone", Solar.FileTextBoldDuotone, Accent),
            Pictogram("solar:toolbox-bold-duotone", Solar.ToolboxBoldDuotone, Accent),
            Pictogram(
                "solar:magic-wand-2-bold-duotone",
                Solar.MagicWand2BoldDuotone,
                Color(0xFF9B7BF7),
            ),
            Pictogram("solar:settings-bold-duotone", Solar.SettingsBoldDuotone, Color(0xFF7D99A3)),
            Pictogram("solar:mailbox-bold-duotone", Solar.MailboxBoldDuotone, Accent),
            Pictogram("solar:keyboard-bold-duotone", Solar.KeyboardBoldDuotone, Color(0xFF7D99A3)),
            Pictogram(
                "solar:camera-minimalistic-bold-duotone",
                Solar.CameraMinimalisticBoldDuotone,
                Accent,
            ),
            Pictogram("solar:album-bold-duotone", Solar.AlbumBoldDuotone, Accent),
        )

    private val byName = all.associateBy { it.name }

    fun host(name: String?): Pictogram = name?.let(byName::get) ?: all[0]

    // Snippets take the accent whatever the icon, so a list of them reads as one.
    fun snippet(name: String?): Pictogram {
        val found = name?.let(byName::get) ?: byName.getValue("solar:file-terminal-bold-duotone")
        return Pictogram(found.name, found.vector, Accent)
    }
}

@Composable
fun PictogramBadge(
    pictogram: Pictogram,
    modifier: Modifier = Modifier,
    size: Dp = 96.dp,
    onClick: (() -> Unit)? = null,
) {
    Box(
        modifier
            .size(size)
            .clip(CircleShape)
            .background(MiuixTheme.colorScheme.surfaceContainer)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier),
        contentAlignment = Alignment.Center,
    ) {
        Icon(pictogram.vector, null, Modifier.size(size * 0.55f), tint = pictogram.tint)
    }
}
