package dev.azuki.mu.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.azuki.mu.Mu
import dev.azuki.mu.api.MuException
import dev.azuki.mu.design.BackButton
import dev.azuki.mu.design.Mono
import dev.azuki.mu.design.Note
import dev.azuki.mu.design.PrimaryButton
import dev.azuki.mu.design.Sheet
import dev.azuki.mu.design.SheetClose
import dev.azuki.mu.design.SheetColumn
import dev.azuki.mu.design.Solar
import dev.azuki.mu.design.icon
import kotlinx.coroutines.launch
import top.yukonga.miuix.kmp.basic.BasicComponent
import top.yukonga.miuix.kmp.basic.Card
import top.yukonga.miuix.kmp.basic.Icon
import top.yukonga.miuix.kmp.basic.SmallTitle
import top.yukonga.miuix.kmp.basic.TextField
import top.yukonga.miuix.kmp.theme.MiuixTheme

@Composable
fun AddHostSheet(show: Boolean, mu: Mu, onDismiss: () -> Unit, invite: String? = null) {
    var link by remember(show, invite) { mutableStateOf(invite.orEmpty()) }
    var pairing by remember { mutableStateOf(false) }
    var scanning by remember(show) { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    fun pair(invite: String) {
        if (pairing) return
        link = invite
        scanning = false
        pairing = true
        scope.launch {
            try {
                val host = mu.pair(invite.trim())
                mu.toast("Paired with ${host.name}")
                onDismiss()
            } catch (e: MuException) {
                mu.toast(e.message)
            } finally {
                pairing = false
            }
        }
    }
    Sheet(
        show = show,
        title = if (scanning) "Scan QR Code" else "Add New Host",
        onDismissRequest = onDismiss,
        startAction = {
            if (scanning) BackButton(onClick = { scanning = false }) else SheetClose()
        },
        bottomAction =
            if (scanning) null
            else {
                {
                    PrimaryButton(
                        text =
                            when {
                                pairing -> "Pairing…"
                                link.isNotBlank() -> "Pair"
                                else -> "Scan QR"
                            },
                        enabled = !pairing,
                    ) {
                        if (link.isNotBlank()) pair(link)
                        else
                            scope.launch {
                                if (mu.platform.allowCamera()) {
                                    scanning = true
                                } else {
                                    mu.toast(
                                        "Scanning needs the camera. Scan with the camera app, or paste the link"
                                    )
                                }
                            }
                    }
                }
            },
    ) { padding ->
        SheetColumn(padding) {
            if (scanning) {
                mu.platform.QrScanner(
                    onScan = ::pair,
                    onError = {
                        scanning = false
                        mu.toast("$it. Scan with the camera app, or paste the link")
                    },
                    modifier =
                        Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(20.dp)),
                )
                Note("Point the camera at the QR code mu pair shows on the host.")
            } else {
                TextField(
                    value = link,
                    onValueChange = { link = it },
                    label = "mu://…",
                    useLabelAsPlaceholder = true,
                    singleLine = true,
                    textStyle =
                        MiuixTheme.textStyles.main.copy(fontFamily = Mono, fontSize = 15.sp),
                )
                Note(
                    "Run mu pair on the host, then scan the QR code it shows, or paste its link above."
                )
                Requirements()
            }
        }
    }
}

@Composable
private fun Requirements() {
    Column {
        SmallTitle(
            "Requirements",
            insideMargin = PaddingValues(start = 12.dp, top = 8.dp, bottom = 8.dp),
        )
        Card {
            BasicComponent(
                title = "Accessible by this Device",
                summary =
                    "Hosts must be accessible by this device over the network. This may mean LAN, Tailscale, WireGuard or similar.",
                startAction = { HelpIcon(Solar.GlobalLineDuotone) },
            )
            BasicComponent(
                title = "mu serve",
                summary =
                    "To advertise as a host. Set MU_LISTEN, like MU_LISTEN=0.0.0.0:7337, to let devices in over the network.",
                startAction = { HelpIcon(Solar.DatabaseBoldDuotone) },
            )
            BasicComponent(
                title = "mu pair",
                summary =
                    "Shows the invite for one device as a QR code and a link. It expires in 10 minutes.",
                startAction = { HelpIcon(Solar.FileTerminalBoldDuotone) },
            )
        }
    }
}

@Composable
private fun HelpIcon(icon: ImageVector) {
    Icon(icon, null, Modifier.size(24.dp), tint = MiuixTheme.colorScheme.primary)
}
