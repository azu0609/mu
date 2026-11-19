package dev.azuki.mu.composer

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.dp
import dev.azuki.mu.Mu
import dev.azuki.mu.chat.Destination
import dev.azuki.mu.design.DialogButtons
import dev.azuki.mu.design.Mono
import dev.azuki.mu.design.Solar
import top.yukonga.miuix.kmp.basic.DropdownEntry
import top.yukonga.miuix.kmp.basic.DropdownItem
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.IconButton
import top.yukonga.miuix.kmp.basic.PopupPositionProvider
import top.yukonga.miuix.kmp.basic.TextField
import top.yukonga.miuix.kmp.theme.LocalDismissState
import top.yukonga.miuix.kmp.theme.MiuixTheme
import top.yukonga.miuix.kmp.window.WindowCascadingListPopup
import top.yukonga.miuix.kmp.window.WindowDialog

val Efforts = listOf("minimal", "low", "medium", "high", "xhigh")

@Composable
internal fun ModelButton(mu: Mu, destination: Destination) {
    var custom by remember { mutableStateOf(false) }
    val model = destination.model
    val effort = destination.effort
    val pick = { picked: String -> destination.use(picked, effort) }
    val models = destination.host?.let { mu.models[it] }.orEmpty()
    val modelItems = buildList {
        if (model != null && models.none { it.name == model }) {
            add(DropdownItem(model, selected = true, onClick = {}))
        }
        models.forEach {
            add(
                DropdownItem(
                    it.name,
                    summary = it.context?.let(mu.core::compact),
                    selected = it.name == model,
                    onClick = { pick(it.name) },
                )
            )
        }
        add(DropdownItem("Other…", onClick = { custom = true }))
    }
    val effortItems =
        listOf(
            DropdownItem(
                "Default",
                selected = effort == null,
                onClick = { destination.use(model, null) },
            )
        ) +
            Efforts.map {
                DropdownItem(it, selected = it == effort, onClick = { destination.use(model, it) })
            }
    val entry =
        DropdownEntry(
            listOf(
                DropdownItem(
                    "Model",
                    summary = model ?: "None",
                    children = modelItems,
                ),
                DropdownItem("Effort", summary = effort ?: "Default", children = effortItems),
            )
        )
    var open by remember { mutableStateOf(false) }
    var held by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    Box {
        IconButton(
            onClick = {
                open = !open
                if (open) {
                    held = true
                    haptics.performHapticFeedback(HapticFeedbackType.ContextClick)
                }
            },
            enabled = destination.ready,
            holdDownState = held,
            minWidth = 36.dp,
            minHeight = 36.dp,
        ) {
            Icon(
                Solar.SpeedometerMaxBoldDuotone,
                "Model and effort",
                Modifier.size(24.dp),
                tint =
                    MiuixTheme.colorScheme.onSurfaceVariantSummary.let {
                        if (destination.ready) it else it.copy(alpha = 0.4f)
                    },
            )
        }
        WindowCascadingListPopup(
            show = open && destination.ready,
            entries = listOf(entry),
            alignment = PopupPositionProvider.Align.Start,
            onDismissRequest = { open = false },
            onDismissFinished = { held = false },
        )
    }
    ModelDialog(custom, onDismiss = { custom = false }, onPick = pick)
}

@Composable
private fun ModelDialog(show: Boolean, onDismiss: () -> Unit, onPick: (String) -> Unit) {
    var name by remember(show) { mutableStateOf("") }
    WindowDialog(
        show = show,
        title = "Model",
        summary = "Any model the host's endpoint serves",
        onDismissRequest = onDismiss,
    ) {
        val dismiss = LocalDismissState.current
        TextField(
            value = name,
            onValueChange = { name = it },
            label = "Model",
            useLabelAsPlaceholder = true,
            singleLine = true,
            textStyle = MiuixTheme.textStyles.main.copy(fontFamily = Mono),
            modifier = Modifier.padding(bottom = 16.dp),
        )
        DialogButtons(
            confirm = "Use",
            enabled = name.isNotBlank(),
            onCancel = { dismiss?.invoke() },
            onConfirm = {
                onPick(name.trim())
                dismiss?.invoke()
            },
        )
    }
}
