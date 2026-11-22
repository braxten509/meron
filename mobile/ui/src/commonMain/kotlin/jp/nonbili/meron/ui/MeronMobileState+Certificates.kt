// Coordinates certificate prompts for mobile account
// operations. A successful trust decision can resume
// the operation that encountered the server error.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountCertPinParams
import jp.nonbili.meron.shared.CertificateProtocol
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ProbeCertParams
import jp.nonbili.meron.shared.ProxySpec
import jp.nonbili.meron.shared.ServerCertificate
import jp.nonbili.meron.shared.parseProbeCertResponse
import jp.nonbili.meron.shared.requireCoreOk
import jp.nonbili.meron.shared.untrustedCertificateProtocol
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The certificate prompt for a save whose servers are not stored yet — a new
 * account, or an edit that failed before it could be written. Same flow as
 * [showServerCertificate], but probing the endpoint the user just typed rather
 * than the account's recorded one, which is stale or absent in both cases.
 *
 * [message] is the failure carrying the core's marker; it names which of the
 * two servers refused. It is restored as the banner if the probe itself fails,
 * so a dead end still explains itself.
 */
internal fun MeronMobileState.showTypedServerCertificate(
    accountId: String,
    imapHost: String,
    imapPort: Int,
    imapSecurity: MailSecurity,
    smtpHost: String,
    smtpPort: Int,
    smtpSecurity: MailSecurity,
    proxy: ProxySpec,
    retry: PendingCertificateRetry,
    message: String,
) {
    val protocol = untrustedCertificateProtocol(message) ?: return
    val smtp = protocol == CertificateProtocol.SMTP
    val host = if (smtp) smtpHost else imapHost
    if (host.isBlank()) return
    val port = if (smtp) smtpPort else imapPort
    val starttls = (if (smtp) smtpSecurity else imapSecurity) == MailSecurity.STARTTLS
    certPromptBusy = true
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                parseProbeCertResponse(
                    requireCoreOk(
                        MobileMailCommandClient(core).probeCert(
                            ProbeCertParams(
                                host = host,
                                port = port,
                                protocol = protocol.wire,
                                starttls = starttls,
                                proxy = proxy,
                            ),
                        ),
                    ),
                )
            }
        }.onSuccess { certificate: ServerCertificate? ->
            if (certificate == null) {
                errorBanner = message
                status = trs("mobile.status.certificateReadFailed")
            } else {
                certPrompt =
                    MobileCertPrompt(
                        accountId = accountId,
                        host = host,
                        port = port,
                        protocol = protocol,
                        certificate = certificate,
                        retry = retry,
                    )
            }
        }.onFailure {
            errorBanner = message
            status = trsError("mobile.status.certificateReadFailed", it)
        }
        certPromptBusy = false
    }
}

/**
 * Fetch the certificate the failing server presented and put it in front of the
 * user. A server whose certificate cannot be validated — a local Proton Mail
 * Bridge serves a self-signed CA certificate as its leaf — is unreachable until
 * that exact certificate is pinned, so the alternative to this prompt is an
 * account that can never sync.
 *
 * [message] is the failure that carries the core's marker; it names which of
 * the two servers refused.
 */
internal fun MeronMobileState.showServerCertificate(
    accountId: String,
    message: String,
    retry: PendingCertificateRetry? = null,
) {
    val protocol = untrustedCertificateProtocol(message) ?: return
    val resolvedAccountId = certificateErrorAccountId(accountId, retry) ?: return
    val account = coreAccounts.find { it.id == resolvedAccountId } ?: return
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val host = if (protocol == CertificateProtocol.SMTP) account.smtpHost else account.imapHost
    if (host.isBlank()) return
    val port =
        when {
            protocol == CertificateProtocol.SMTP -> account.smtpPort.takeIf { it > 0 } ?: 465
            else -> account.imapPort.takeIf { it > 0 } ?: 993
        }
    val starttls = if (protocol == CertificateProtocol.SMTP) account.smtpStarttls else account.starttls
    certPromptBusy = true
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                parseProbeCertResponse(
                    requireCoreOk(
                        MobileMailCommandClient(core).probeCert(
                            ProbeCertParams(
                                host = host,
                                port = port,
                                protocol = protocol.wire,
                                starttls = starttls,
                                proxy = account.proxy,
                            ),
                        ),
                    ),
                )
            }
        }.onSuccess { certificate: ServerCertificate? ->
            if (certificate == null) {
                status = trs("mobile.status.certificateReadFailed")
            } else {
                certPrompt =
                    MobileCertPrompt(
                        accountId = resolvedAccountId,
                        host = host,
                        port = port,
                        protocol = protocol,
                        certificate = certificate,
                        retry = retry,
                    )
            }
        }.onFailure {
            status = trsError("mobile.status.certificateReadFailed", it)
        }
        certPromptBusy = false
    }
}

/**
 * Pin the certificate the user accepted and retry the sync. Only the server the
 * prompt was about is pinned: the other one keeps whatever it had.
 */
internal fun MeronMobileState.trustPromptedCertificate() {
    val prompt = certPrompt ?: return
    val fingerprint = prompt.certificate.fingerprint
    // Pinning normally writes to the account's row and lets the retry read it
    // back. An account that does not exist yet has no row, so its pin travels
    // on the request that creates it instead.
    val addRetry = prompt.retry as? PendingCertificateRetry.AddAccount
    if (addRetry != null) {
        certPrompt = null
        errorBanner = null
        if (pendingCertificateRetry == prompt.retry) pendingCertificateRetry = null
        status = trs("mobile.status.certificateTrusted", mapOf("host" to prompt.host))
        val smtp = prompt.protocol == CertificateProtocol.SMTP
        addPasswordAccount(
            addRetry.params.copy(
                certPin = if (smtp) addRetry.params.certPin else fingerprint,
                smtpCertPin = if (smtp) fingerprint else addRetry.params.smtpCertPin,
            ),
        )
        return
    }
    certPromptBusy = true
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                requireCoreOk(
                    MobileMailCommandClient(core).setAccountCertPin(
                        AccountCertPinParams(
                            accountId = prompt.accountId,
                            certPin = fingerprint.takeIf { prompt.protocol == CertificateProtocol.IMAP },
                            smtpCertPin = fingerprint.takeIf { prompt.protocol == CertificateProtocol.SMTP },
                        ),
                    ),
                )
            }
        }.onSuccess {
            certPrompt = null
            syncError = null
            errorBanner = null
            status = trs("mobile.status.certificateTrusted", mapOf("host" to prompt.host))
            // Resume what the certificate blocked — an unsent message stays
            // unsent unless its send is the thing that runs again.
            if (pendingCertificateRetry == prompt.retry) pendingCertificateRetry = null
            when (val retry = prompt.retry) {
                is PendingCertificateRetry.Compose -> {
                    retryComposeSend(retry.pending)
                }

                is PendingCertificateRetry.QuickReply -> {
                    retryQuickReplySend(retry.pending)
                }

                is PendingCertificateRetry.ServerSettings -> {
                    val account = coreAccounts.find { it.id == retry.accountId }
                    if (account == null) syncCoreThreads() else saveAccountServerSettings(account, retry.draft)
                }

                // Handled by the early return above: an account that does not
                // exist yet never reaches the pin-then-retry path.
                is PendingCertificateRetry.AddAccount -> {
                    Unit
                }

                null -> {
                    syncCoreThreads()
                }
            }
        }.onFailure {
            status = trsError("mobile.status.certificateSaveFailed", it)
        }
        certPromptBusy = false
    }
}

internal fun MeronMobileState.dismissCertificatePrompt() {
    certPrompt = null
}
