// Creates or reconnects a password-based account from the mobile setup form.
// Discovery fills candidate server settings and the form tracks certificate decisions.
// The submitted settings and the selected transport reach the core together.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AddPasswordAccountParams
import jp.nonbili.meron.shared.AutodiscoverAccountParams
import jp.nonbili.meron.shared.CertificateProtocol
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ProxySpec
import jp.nonbili.meron.shared.parseAutodiscoverResponse
import jp.nonbili.meron.shared.untrustedCertificateProtocol
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun MeronMobileState.addPasswordAccount() {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    addPasswordAccount(
        AddPasswordAccountParams(
            email = email.trim(),
            displayName = displayName.trim(),
            senderName = senderName.trim(),
            imapHost = host.trim(),
            imapPort = imapPort.trim().toIntOrNull() ?: 993,
            smtpHost = smtpHost.trim(),
            smtpPort = smtpPort.trim().toIntOrNull() ?: 465,
            username = username.trim().ifBlank { email.trim() },
            password = password,
            tls = imapSecurity == MailSecurity.TLS,
            starttls = imapSecurity == MailSecurity.STARTTLS,
            smtpTls = smtpSecurity == MailSecurity.TLS,
            smtpStarttls = smtpSecurity == MailSecurity.STARTTLS,
        ),
    )
}

/**
 * Create the account described by [params].
 *
 * A server whose certificate cannot be validated — a local Proton Mail Bridge serves a self-signed CA
 * certificate as its leaf — is offered for pinning instead of dead-ending on the handshake error,
 * which is what setup used to do: the banner named a TLS failure the user had no way to act on, so
 * such an account simply could not be added on this device.
 *
 * Accepting re-enters here with the pin attached, since there is no stored account to write it to yet.
 */
internal fun MeronMobileState.addPasswordAccount(params: AddPasswordAccountParams) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    status = trs("mobile.status.addingPasswordAccount")
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                client.addPasswordAccount(params)
                client.listAccounts()
            }
        }.onSuccess {
            applyAccounts(it, preferEmail = params.email)
            resetPasswordAccountForm()
            screen = Screen.Mail
            errorBanner = null
            certPrompt = null
            status = trs("mobile.status.accountAdded", mapOf("account" to params.email))
            syncCoreThreads(accountOverride = selectedCoreAccountId, folderOverride = INBOX_FOLDER, syncFirst = true)
        }.onFailure { failure ->
            val message = failure.message ?: trs("mobile.status.addAccountFailed")
            if (untrustedCertificateProtocol(message) != null &&
                shouldPromptForNewCertificate(params, message)
            ) {
                showTypedServerCertificate(
                    accountId = mailAccountId(params.email),
                    imapHost = params.imapHost,
                    imapPort = params.imapPort,
                    imapSecurity = mailSecurityOf(params.tls, params.starttls ?: false),
                    smtpHost = params.smtpHost,
                    smtpPort = params.smtpPort,
                    smtpSecurity = mailSecurityOf(params.smtpTls ?: true, params.smtpStarttls ?: false),
                    proxy = ProxySpec.followApp,
                    retry = PendingCertificateRetry.AddAccount(mailAccountId(params.email), params),
                    message = message,
                )
            } else {
                errorBanner = message
                status = withReason(trs("mobile.status.addAccountFailed"), failure.message) { key, args -> trs(key, args) }
            }
        }
    }
}

/**
 * The id the core mints for a mail address, so a prompt raised before the
 * account exists still names the account it will become. Mirrors `accountID`
 * in the bridge: the normalized address, used verbatim.
 */
internal fun mailAccountId(email: String): String = email.trim().lowercase()

/**
 * Whether a certificate failure is worth prompting about, or is the *same*
 * refusal we already pinned for. Retrying a pin that did not help would
 * otherwise loop the prompt forever.
 */
private fun shouldPromptForNewCertificate(
    params: AddPasswordAccountParams,
    message: String,
): Boolean =
    when (untrustedCertificateProtocol(message)) {
        CertificateProtocol.SMTP -> params.smtpCertPin == null
        else -> params.certPin == null
    }

// Clear the password setup form when its account is added and whenever a fresh
// setup starts, so an account never inherits the previous server, ports and
// security modes -- a sticky "touched" flag would otherwise keep autodiscovery
// and port edits from correcting a hand-picked mode.
internal fun MeronMobileState.resetPasswordAccountForm() {
    email = ""
    username = ""
    password = ""
    displayName = ""
    senderName = ""
    host = ""
    hostTouched = false
    imapPort = "993"
    imapPortTouched = false
    imapSecurity = MailSecurity.TLS
    imapSecurityTouched = false
    smtpHost = ""
    smtpHostTouched = false
    smtpPort = "465"
    smtpPortTouched = false
    smtpSecurity = MailSecurity.TLS
    smtpSecurityTouched = false
    lastAutodiscoverEmail = ""
    passwordAutodiscoverGeneration += 1
    passwordServerSettingsOpen = false
}

internal fun MeronMobileState.autodiscoverPasswordAccount(auto: Boolean = false) {
    val emailValue = email.trim()
    if (!emailValue.contains('@') || emailValue.endsWith('@')) {
        // Don't nag while the user is still typing the address.
        if (!auto) status = trs("mobile.status.enterEmailFirst")
        return
    }
    // The on-blur trigger fires whenever focus leaves the email field; skip the
    // lookup unless the address actually changed since the last attempt.
    if (auto && emailValue.equals(lastAutodiscoverEmail, ignoreCase = true)) return
    lastAutodiscoverEmail = emailValue
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    passwordAutodiscoverGeneration += 1
    val requestGeneration = passwordAutodiscoverGeneration
    status = trs("accounts.discovery.lookingUp")
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                parseAutodiscoverResponse(client.autodiscoverAccount(AutodiscoverAccountParams(emailValue)))
            }
        }.onSuccess { discovered ->
            if (
                requestGeneration != passwordAutodiscoverGeneration ||
                !email.trim().equals(emailValue, ignoreCase = true)
            ) {
                return@onSuccess
            }
            val discoveredImapPort = discovered.imapPort.takeIf { discovered.imapHost.isNotBlank() } ?: 0
            val discoveredSmtpPort = discovered.smtpPort.takeIf { discovered.smtpHost.isNotBlank() } ?: 0
            val imapSelection =
                mailServerSelectionAfterDiscovery(
                    imapPort,
                    imapSecurity,
                    imapSecurityTouched,
                    hostTouched,
                    imapPortTouched,
                    discoveredImapPort,
                    preserveUserSettings = auto,
                )
            val smtpSelection =
                mailServerSelectionAfterDiscovery(
                    smtpPort,
                    smtpSecurity,
                    smtpSecurityTouched,
                    smtpHostTouched,
                    smtpPortTouched,
                    discoveredSmtpPort,
                    preserveUserSettings = auto,
                )
            if ((!auto || !hostTouched) && discovered.imapHost.isNotBlank()) {
                host = discovered.imapHost
                if (!auto) hostTouched = false
            }
            imapPort = imapSelection.port
            imapSecurity = imapSelection.security
            if (!auto && discoveredServerIsComplete(discovered.imapHost, discovered.imapPort)) {
                imapPortTouched = false
                imapSecurityTouched = false
            }
            if ((!auto || !smtpHostTouched) && discovered.smtpHost.isNotBlank()) {
                smtpHost = discovered.smtpHost
                if (!auto) smtpHostTouched = false
            }
            smtpPort = smtpSelection.port
            smtpSecurity = smtpSelection.security
            if (!auto && discoveredServerIsComplete(discovered.smtpHost, discovered.smtpPort)) {
                smtpPortTouched = false
                smtpSecurityTouched = false
            }
            if (discovered.username.isNotBlank()) username = discovered.username
            status =
                when {
                    discovered.appPasswordProvider.isNotBlank() -> {
                        trs(
                            "mobile.ios.settingsFoundUseAppPassword",
                            mapOf("provider" to discovered.providerName.ifBlank { discovered.appPasswordProvider }),
                        )
                    }

                    discovered.source == "guess" -> {
                        passwordServerSettingsOpen = true
                        trs("mobile.status.settingsGuessed")
                    }

                    else -> {
                        discovered.providerName.takeIf { it.isNotBlank() }?.let { trs("mobile.ios.settingsFoundForProvider", mapOf("provider" to it)) }
                            ?: trs("mobile.ios.settingsFound")
                    }
                }
        }.onFailure {
            if (
                requestGeneration != passwordAutodiscoverGeneration ||
                !email.trim().equals(emailValue, ignoreCase = true)
            ) {
                return@onFailure
            }
            passwordServerSettingsOpen = true
            status = trsError("mobile.status.settingsLookupFailed", it)
        }
    }
}

internal fun mailSecurityForPort(port: Int): MailSecurity =
    when (port) {
        25, 143, 587 -> MailSecurity.STARTTLS
        3143, 3587 -> MailSecurity.NONE
        else -> MailSecurity.TLS
    }

internal fun mailSecurityAfterPortEdit(
    current: MailSecurity,
    touched: Boolean,
    portText: String,
): MailSecurity {
    if (touched) return current
    val port = portText.toIntOrNull()?.takeIf { it > 0 } ?: return current
    return mailSecurityForPort(port)
}

internal data class MailServerSelection(
    val port: String,
    val security: MailSecurity,
)

internal fun discoveredServerIsComplete(
    host: String,
    port: Int,
): Boolean = host.isNotBlank() && port > 0

internal fun mailServerSelectionAfterDiscovery(
    currentPort: String,
    currentSecurity: MailSecurity,
    securityTouched: Boolean,
    hostTouched: Boolean,
    portTouched: Boolean,
    discoveredPort: Int,
    preserveUserSettings: Boolean = true,
): MailServerSelection =
    if ((preserveUserSettings && (hostTouched || portTouched)) || discoveredPort <= 0) {
        MailServerSelection(currentPort, currentSecurity)
    } else {
        MailServerSelection(
            discoveredPort.toString(),
            if (preserveUserSettings && securityTouched) currentSecurity else mailSecurityForPort(discoveredPort),
        )
    }
