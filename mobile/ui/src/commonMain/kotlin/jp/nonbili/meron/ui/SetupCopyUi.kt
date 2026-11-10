package jp.nonbili.meron.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ArrowForward
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Smartphone
import androidx.compose.material.icons.filled.SwapHoriz
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put

// "Copy setup to another device", the phone's side: scan the computer's QR
// code (or type its address and code), then the same Keep/Erased screen the
// computer shows, with a swap button and Copy. The core does the networking
// and the encryption; this page polls its status.

private const val SETUP_POLL_MS = 500L

private val SETUP_ERRORS =
    setOf(
        "no_network",
        "expired",
        "too_many_attempts",
        "wrong_code",
        "bad_qr",
        "bad_address",
        "bad_code",
        "refused",
        "other_network",
        "unreachable",
        "peer_left",
        "peer_failed",
        "peer_bad_copy",
        "bad_copy",
        "safety_failed",
        "apply_failed",
        "collect_failed",
    )

private data class SetupCounts(
    val accounts: Int = 0,
    val folders: Int = 0,
    val keys: Int = 0,
)

private data class SetupStatus(
    val stage: String = "idle",
    val me: String = "phone",
    val keep: String = "",
    val mine: SetupCounts = SetupCounts(),
    val peer: SetupCounts? = null,
    val starting: Boolean = false,
    val error: String = "",
    val replaced: Boolean = false,
    val secretFailures: Int = 0,
) {
    val active: Boolean get() = stage !in setOf("idle", "done", "failed")
}

private data class SafetyCopy(
    val exists: Boolean = false,
    val createdAt: Long = 0,
)

private fun JsonObject.str(key: String): String = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content.orEmpty()

private fun JsonObject.int(key: String): Int = (this[key] as? JsonPrimitive)?.intOrNull ?: 0

private fun JsonObject.bool(key: String): Boolean = (this[key] as? JsonPrimitive)?.booleanOrNull ?: false

private fun parseCounts(value: Any?): SetupCounts? {
    val obj = value as? JsonObject ?: return null
    return SetupCounts(obj.int("accounts"), obj.int("folders"), obj.int("keys"))
}

private fun parseSetupStatus(json: String): SetupStatus {
    val obj = runCatching { Json.parseToJsonElement(json).jsonObject }.getOrNull() ?: return SetupStatus()
    val result = obj["result"] as? JsonObject
    return SetupStatus(
        stage = obj.str("stage").ifEmpty { "idle" },
        me = obj.str("me").ifEmpty { "phone" },
        keep = obj.str("keep"),
        mine = parseCounts(obj["mine"]) ?: SetupCounts(),
        peer = parseCounts(obj["peer"]),
        starting = obj.bool("starting"),
        error = obj.str("error"),
        replaced = obj.bool("replaced"),
        secretFailures = result?.int("secret_failures") ?: 0,
    )
}

private suspend fun MeronMobileState.setupCopyCall(
    command: String,
    params: JsonObject = JsonObject(emptyMap()),
): String = withContext(ioDispatcher) { core.invoke("setupCopy.$command", params.toString()) }

private suspend fun MeronMobileState.setupCopyStatus(
    command: String,
    params: JsonObject = JsonObject(emptyMap()),
): SetupStatus? = runCatching { parseSetupStatus(setupCopyCall(command, params)) }.getOrNull()

private suspend fun MeronMobileState.loadSafetyCopy(): SafetyCopy =
    runCatching {
        val obj = Json.parseToJsonElement(setupCopyCall("safetyInfo")).jsonObject
        SafetyCopy(
            exists = obj.bool("exists"),
            createdAt = (obj["created_at"] as? JsonPrimitive)?.longOrNull ?: 0,
        )
    }.getOrDefault(SafetyCopy())

@Composable
private fun setupErrorText(code: String): String = tr(if (code in SETUP_ERRORS) "setupCopy.error.$code" else "setupCopy.error.broken")

@Composable
internal fun MeronMobileState.SetupCopyPage(modifier: Modifier = Modifier) {
    val platform = LocalPlatformServices.current
    var status by remember { mutableStateOf(SetupStatus()) }
    var typing by remember { mutableStateOf(false) }
    var address by remember { mutableStateOf("") }
    var code by remember { mutableStateOf("") }
    var note by remember { mutableStateOf("") }
    var confirmCopy by remember { mutableStateOf(false) }
    var safety by remember { mutableStateOf(SafetyCopy()) }
    var confirmRestore by remember { mutableStateOf(false) }
    var restoring by remember { mutableStateOf(false) }
    var reloaded by remember { mutableStateOf(false) }
    val coroutines = rememberCoroutineScope()
    val scanPrompt = tr("setupCopy.scanPrompt")
    val cameraDeniedText = tr("setupCopy.cameraDenied")
    val cannotScanText = tr("setupCopy.cannotScan")
    val restoredText = tr("setupCopy.restored")
    val restoreFailedText = tr("setupCopy.restoreFailed")

    LaunchedEffect(Unit) {
        setupCopyStatus("status")?.let { status = it }
        safety = loadSafetyCopy()
        while (true) {
            delay(SETUP_POLL_MS)
            setupCopyStatus("status")?.let { status = it }
        }
    }
    // Leaving the page ends a session still open, telling the computer.
    DisposableEffect(Unit) {
        onDispose {
            scope.launch { runCatching { setupCopyCall("cancel") } }
        }
    }
    LaunchedEffect(status.stage, status.replaced) {
        if (status.stage == "done" && status.replaced && !reloaded) {
            reloaded = true
            reloadAfterSetupReplaced()
            safety = loadSafetyCopy()
        }
    }

    fun join(params: JsonObject) {
        note = ""
        reloaded = false
        coroutines.launch { setupCopyStatus("join", params)?.let { status = it } }
    }

    Column(
        modifier.verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        when (status.stage) {
            "connecting" -> {
                Busy(tr("setupCopy.connecting"))
            }

            "connected", "copying", "applying" -> {
                DirectionScreen(
                    status = status,
                    onSwap = {
                        val other = if (status.keep == "phone") "computer" else "phone"
                        coroutines.launch {
                            setupCopyStatus("setKeep", buildJsonObject { put("keep", other) })?.let { status = it }
                        }
                    },
                    onCopy = { confirmCopy = true },
                )
            }

            "done" -> {
                Surface(
                    color = MaterialTheme.colorScheme.primary.copy(alpha = 0.1f),
                    shape = RoundedCornerShape(16.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Icon(Icons.Filled.CheckCircle, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
                        Text(tr(if (status.replaced) "setupCopy.doneReceived" else "setupCopy.doneSent"))
                    }
                }
                if (status.replaced && status.secretFailures > 0) {
                    Text(
                        tr("setupCopy.secretFailures", mapOf("count" to status.secretFailures)),
                        color = MaterialTheme.colorScheme.error,
                        fontSize = 13.sp,
                    )
                }
                OutlinedButton(
                    onClick = { coroutines.launch { setupCopyStatus("cancel")?.let { status = it } } },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(tr("buttons.done")) }
            }

            else -> {
                if (status.stage == "failed") {
                    Surface(
                        color = MaterialTheme.colorScheme.errorContainer,
                        shape = RoundedCornerShape(16.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Icon(Icons.Filled.Warning, contentDescription = null, tint = MaterialTheme.colorScheme.error)
                            Text(setupErrorText(status.error), color = MaterialTheme.colorScheme.onErrorContainer)
                        }
                    }
                }
                Text(tr("setupCopy.phoneHint"), color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 14.sp)
                Button(
                    onClick = {
                        note = ""
                        platform.scanQrCode(scanPrompt) { result ->
                            when {
                                result.text != null -> join(buildJsonObject { put("qr", result.text) })
                                result.cameraDenied -> {
                                    note = cameraDeniedText
                                    typing = true
                                }
                                result.unsupported -> {
                                    note = cannotScanText
                                    typing = true
                                }
                            }
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(Icons.Filled.QrCodeScanner, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.size(8.dp))
                    Text(tr("setupCopy.scan"))
                }
                if (note.isNotEmpty()) {
                    Text(note, color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 13.sp)
                }
                if (!typing) {
                    TextButton(onClick = { typing = true }, modifier = Modifier.fillMaxWidth()) {
                        Text(tr("setupCopy.typeInstead"))
                    }
                } else {
                    OutlinedTextField(
                        value = address,
                        onValueChange = { address = it.trim() },
                        label = { Text(tr("setupCopy.address")) },
                        placeholder = { Text("192.168.1.20:41234") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = code,
                        onValueChange = { code = it.uppercase().take(9) },
                        label = { Text(tr("setupCopy.code")) },
                        placeholder = { Text("ABCD-EFGH") },
                        singleLine = true,
                        keyboardOptions =
                            KeyboardOptions(
                                capitalization = KeyboardCapitalization.Characters,
                                keyboardType = KeyboardType.Ascii,
                            ),
                        textStyle = MaterialTheme.typography.bodyLarge.copy(fontFamily = FontFamily.Monospace),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Button(
                        onClick = {
                            join(
                                buildJsonObject {
                                    put("address", address)
                                    put("code", code)
                                },
                            )
                        },
                        enabled = address.isNotBlank() && code.isNotBlank(),
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(tr("setupCopy.connect")) }
                }

                if (safety.exists) {
                    Spacer(Modifier.height(8.dp))
                    Text(tr("setupCopy.safetyTitle"), style = MaterialTheme.typography.titleSmall)
                    Text(
                        tr("setupCopy.safetyHint", mapOf("date" to formatSafetyDate(safety.createdAt))),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 13.sp,
                    )
                    OutlinedButton(
                        onClick = { confirmRestore = true },
                        enabled = !restoring,
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(tr("setupCopy.restoreSafety")) }
                }
            }
        }
    }

    if (confirmCopy) {
        val erased = if (status.keep == "phone") "computer" else "phone"
        AlertDialog(
            onDismissRequest = { confirmCopy = false },
            title = { Text(tr(if (erased == "phone") "setupCopy.confirmErasePhone" else "setupCopy.confirmEraseComputer")) },
            text = { Text(tr("setupCopy.confirmBody")) },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmCopy = false
                        coroutines.launch { setupCopyStatus("copy")?.let { status = it } }
                    },
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) { Text(tr("setupCopy.confirmAction")) }
            },
            dismissButton = { TextButton(onClick = { confirmCopy = false }) { Text(tr("buttons.cancel")) } },
        )
    }

    if (confirmRestore) {
        AlertDialog(
            onDismissRequest = { if (!restoring) confirmRestore = false },
            title = { Text(tr("setupCopy.restoreSafety")) },
            text = { Text(tr("setupCopy.restoreConfirm", mapOf("date" to formatSafetyDate(safety.createdAt)))) },
            confirmButton = {
                TextButton(
                    enabled = !restoring,
                    onClick = {
                        restoring = true
                        coroutines.launch {
                            val ok = runCatching { setupCopyCall("restoreSafety") }.isSuccess
                            if (ok) reloadAfterSetupReplaced()
                            safety = loadSafetyCopy()
                            note = if (ok) restoredText else restoreFailedText
                            restoring = false
                            confirmRestore = false
                        }
                    },
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
                ) { Text(tr("setupCopy.restoreAction")) }
            },
            dismissButton = {
                TextButton(enabled = !restoring, onClick = { confirmRestore = false }) { Text(tr("buttons.cancel")) }
            },
        )
    }
}

/** A safety copy's date, as the rest of the app shows dates. */
private fun formatSafetyDate(seconds: Long): String = formatMessageFullTimestamp(seconds)

@Composable
private fun Busy(text: String) {
    Row(
        Modifier.fillMaxWidth().padding(vertical = 32.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
        Spacer(Modifier.size(12.dp))
        Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun countsText(counts: SetupCounts?): Triple<String, String, String> {
    val c = counts ?: SetupCounts()
    return Triple(
        tr("setupCopy.accounts", mapOf("count" to c.accounts)),
        tr("setupCopy.folders", mapOf("count" to c.folders)),
        tr("setupCopy.keys", mapOf("count" to c.keys)),
    )
}

@Composable
private fun DirectionScreen(
    status: SetupStatus,
    onSwap: () -> Unit,
    onCopy: () -> Unit,
) {
    val keep = status.keep.ifEmpty { "computer" }
    val erased = if (keep == "phone") "computer" else "phone"
    fun countsOf(device: String) = if (status.me == device) status.mine else status.peer
    val working = status.stage == "copying" || status.stage == "applying"
    val locked = working || status.starting
    val (accounts, folders, keys) = countsText(countsOf(erased))

    Row(
        Modifier.fillMaxWidth().height(IntrinsicSize.Min),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        DeviceBox(Icons.Filled.Smartphone, tr("setupCopy.phone"), keep == "phone", countsOf("phone"), status.me == "phone", Modifier.weight(1f))
        Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.padding(horizontal = 4.dp)) {
            Icon(
                if (keep == "phone") Icons.AutoMirrored.Filled.ArrowForward else Icons.AutoMirrored.Filled.ArrowBack,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
            )
            IconButton(onClick = onSwap, enabled = !locked) {
                Icon(Icons.Filled.SwapHoriz, contentDescription = tr("setupCopy.swap"))
            }
        }
        DeviceBox(Icons.Filled.Computer, tr("setupCopy.computer"), keep == "computer", countsOf("computer"), status.me == "computer", Modifier.weight(1f))
    }
    Text(
        tr(
            if (keep == "phone") "setupCopy.summaryPhoneKept" else "setupCopy.summaryComputerKept",
            mapOf("accounts" to accounts, "folders" to folders, "keys" to keys),
        ),
        textAlign = TextAlign.Center,
        modifier = Modifier.fillMaxWidth(),
    )
    if (working) {
        Busy(tr(if (status.stage == "applying") "setupCopy.applying" else "setupCopy.copying"))
    } else {
        Button(onClick = onCopy, enabled = !locked, modifier = Modifier.fillMaxWidth()) {
            if (status.starting) {
                CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onPrimary)
                Spacer(Modifier.size(8.dp))
            }
            Text(tr("setupCopy.copy"))
        }
    }
}

@Composable
private fun DeviceBox(
    icon: ImageVector,
    label: String,
    kept: Boolean,
    counts: SetupCounts?,
    me: Boolean,
    modifier: Modifier = Modifier,
) {
    val colors = MaterialTheme.colorScheme
    val (accounts, folders, keys) = countsText(counts)
    Surface(
        shape = RoundedCornerShape(16.dp),
        color = if (kept) colors.primary.copy(alpha = 0.08f) else colors.surfaceVariant.copy(alpha = 0.5f),
        border = BorderStroke(1.dp, if (kept) colors.primary.copy(alpha = 0.4f) else colors.outlineVariant),
        modifier = modifier.fillMaxHeight(),
    ) {
        Column(
            Modifier.padding(horizontal = 8.dp, vertical = 12.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(icon, contentDescription = null, tint = if (kept) colors.primary else colors.onSurfaceVariant)
            Text(label, fontWeight = FontWeight.Bold, fontSize = 14.sp)
            if (me) Text(tr("setupCopy.thisDevice"), fontSize = 11.sp, color = colors.onSurfaceVariant)
            Box(
                Modifier.padding(top = 2.dp),
            ) {
                Surface(
                    shape = RoundedCornerShape(50),
                    color = if (kept) colors.primary else colors.errorContainer,
                ) {
                    Text(
                        tr(if (kept) "setupCopy.keep" else "setupCopy.erased"),
                        color = if (kept) colors.onPrimary else colors.onErrorContainer,
                        fontSize = 11.sp,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 2.dp),
                    )
                }
            }
            Text(
                "$accounts · $folders · $keys",
                fontSize = 11.sp,
                color = colors.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
    }
}

/**
 * First-run shortcut under the welcome screen's "Add account": set this phone
 * up from a computer that already runs Meron, through the same QR copy as
 * Settings. The copy keeps whichever side has accounts, so an empty phone
 * receives by default. Once the copy lands the welcome screen (and this) goes
 * away on its own.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun MeronMobileState.CopySetupFromComputerLink() {
    var open by remember { mutableStateOf(false) }
    Text(
        tr("accounts.setup.copyFromComputer"),
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        textAlign = TextAlign.Center,
    )
    TextButton(onClick = { open = true }) {
        Text(tr("accounts.setup.copyFromPhoneLink"), fontWeight = FontWeight.SemiBold)
    }
    if (open) {
        Dialog(
            onDismissRequest = { open = false },
            properties = DialogProperties(usePlatformDefaultWidth = false),
        ) {
            Scaffold(
                topBar = {
                    TopAppBar(
                        title = { Text(tr("setupCopy.title")) },
                        navigationIcon = {
                            IconButton(onClick = { open = false }) {
                                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = tr("buttons.back"))
                            }
                        },
                    )
                },
            ) { padding ->
                SetupCopyPage(Modifier.fillMaxSize().padding(padding))
            }
        }
    }
}
