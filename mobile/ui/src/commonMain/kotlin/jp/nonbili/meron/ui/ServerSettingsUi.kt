// Edits mail server details inside mobile account
// settings. Stored flags and user selections
// determine the transport security shown in the
// form.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Dns
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.AccountSummary

/** The servers an account connects through, as the editor hands them back. */
internal data class ServerSettingsDraft(
    val imapHost: String,
    val imapPort: Int,
    val imapSecurity: MailSecurity,
    val smtpHost: String,
    val smtpPort: Int,
    val smtpSecurity: MailSecurity,
    val username: String,
    /** Null keeps the stored password; only a typed one replaces it. */
    val password: String?,
)

internal fun mailSecurityOf(
    tls: Boolean,
    starttls: Boolean,
): MailSecurity =
    when {
        starttls -> MailSecurity.STARTTLS
        tls -> MailSecurity.TLS
        else -> MailSecurity.NONE
    }

@Composable
private fun securityLabel(security: MailSecurity): String =
    when (security) {
        MailSecurity.TLS -> "TLS"
        MailSecurity.STARTTLS -> "STARTTLS"
        MailSecurity.NONE -> tr("accounts.security.none")
    }

/** Row subtitle: both endpoints on one line, so the row says what is configured. */
@Composable
private fun serverSummary(account: AccountSummary): String {
    val imapPort = account.imapPort.takeIf { it > 0 } ?: 993
    val smtpPort = account.smtpPort.takeIf { it > 0 } ?: 465
    val imap =
        "${account.imapHost.ifBlank { "—" }}:$imapPort (${securityLabel(mailSecurityOf(account.tls, account.starttls))})"
    val smtp =
        "${account.smtpHost.ifBlank { "—" }}:$smtpPort (${securityLabel(mailSecurityOf(account.smtpTls, account.smtpStarttls))})"
    return "IMAP $imap · SMTP $smtp"
}

/**
 * The account's server row plus its editor, mirroring [SettingsProxyRow].
 *
 * Saving is explicit rather than per-keystroke: a half-typed host pushed to the
 * core would break the account's next connection. The password field stays
 * blank and only travels when the user types one — the UI never holds the
 * stored credential, and sending an empty string would clear it.
 */
@Composable
internal fun SettingsServerRow(
    account: AccountSummary,
    onSave: (ServerSettingsDraft) -> Unit,
) {
    var editing by remember { mutableStateOf(false) }
    SettingsRow(
        icon = Icons.Filled.Dns,
        title = tr("settings.account.serverAccount"),
        subtitle = serverSummary(account),
        onClick = { editing = true },
        trailing = { Text(tr("settings.account.serverEdit"), color = MaterialTheme.colorScheme.primary) },
    )
    if (editing) {
        ServerSettingsEditorDialog(
            account = account,
            onSave = {
                onSave(it)
                editing = false
            },
            onDismiss = { editing = false },
        )
    }
}

@Composable
private fun ServerSettingsEditorDialog(
    account: AccountSummary,
    onSave: (ServerSettingsDraft) -> Unit,
    onDismiss: () -> Unit,
) {
    var imapHost by remember { mutableStateOf(account.imapHost) }
    var imapPortText by remember { mutableStateOf((account.imapPort.takeIf { it > 0 } ?: 993).toString()) }
    var imapSecurity by remember { mutableStateOf(mailSecurityOf(account.tls, account.starttls)) }
    var smtpHost by remember { mutableStateOf(account.smtpHost) }
    var smtpPortText by remember { mutableStateOf((account.smtpPort.takeIf { it > 0 } ?: 465).toString()) }
    var smtpSecurity by remember { mutableStateOf(mailSecurityOf(account.smtpTls, account.smtpStarttls)) }
    var username by remember { mutableStateOf(account.username.ifBlank { account.email }) }
    var password by remember { mutableStateOf("") }

    val imapPort = imapPortText.toIntOrNull() ?: 0
    val smtpPort = smtpPortText.toIntOrNull() ?: 0
    val canSave = imapHost.isNotBlank() && smtpHost.isNotBlank() && imapPort in 1..65535 && smtpPort in 1..65535

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr("settings.account.serverTitle")) },
        text = {
            val scrollState = rememberScrollState()
            Column(modifier = Modifier.heightIn(max = 480.dp).appScrollbar(scrollState).verticalScroll(scrollState)) {
                OutlinedTextField(
                    value = imapHost,
                    onValueChange = { imapHost = it.trim() },
                    label = { Text(tr("accounts.fields.imapHost")) },
                    singleLine = true,
                    keyboardOptions = nativeTextKeyboardOptions,
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
                OutlinedTextField(
                    value = imapPortText,
                    onValueChange = { imapPortText = it.filter(Char::isDigit).take(5) },
                    label = { Text(tr("accounts.fields.imapPort")) },
                    singleLine = true,
                    keyboardOptions = nativeTextKeyboardOptions.copy(keyboardType = KeyboardType.Number),
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
                MailSecuritySelector(
                    label = tr("accounts.fields.imapSecurity"),
                    security = imapSecurity,
                    onSecurityChange = { imapSecurity = it },
                )
                OutlinedTextField(
                    value = smtpHost,
                    onValueChange = { smtpHost = it.trim() },
                    label = { Text(tr("accounts.fields.smtpHost")) },
                    singleLine = true,
                    keyboardOptions = nativeTextKeyboardOptions,
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
                OutlinedTextField(
                    value = smtpPortText,
                    onValueChange = { smtpPortText = it.filter(Char::isDigit).take(5) },
                    label = { Text(tr("accounts.fields.smtpPort")) },
                    singleLine = true,
                    keyboardOptions = nativeTextKeyboardOptions.copy(keyboardType = KeyboardType.Number),
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
                MailSecuritySelector(
                    label = tr("accounts.fields.smtpSecurity"),
                    security = smtpSecurity,
                    onSecurityChange = { smtpSecurity = it },
                )
                OutlinedTextField(
                    value = username,
                    onValueChange = { username = it.trim() },
                    label = { Text(tr("accounts.fields.username")) },
                    singleLine = true,
                    keyboardOptions = nativeTextKeyboardOptions,
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
                OutlinedTextField(
                    value = password,
                    onValueChange = { password = it },
                    label = { Text(tr("accounts.fields.passwordUnchanged")) },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    keyboardOptions = nativeTextKeyboardOptions.copy(keyboardType = KeyboardType.Password),
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
            }
        },
        confirmButton = {
            TextButton(
                enabled = canSave,
                onClick = {
                    onSave(
                        ServerSettingsDraft(
                            imapHost = imapHost.trim(),
                            imapPort = imapPort,
                            imapSecurity = imapSecurity,
                            smtpHost = smtpHost.trim(),
                            smtpPort = smtpPort,
                            smtpSecurity = smtpSecurity,
                            username = username.trim(),
                            password = password.takeIf { it.isNotEmpty() },
                        ),
                    )
                },
            ) { Text(tr("buttons.save")) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(tr("buttons.cancel")) } },
    )
}
