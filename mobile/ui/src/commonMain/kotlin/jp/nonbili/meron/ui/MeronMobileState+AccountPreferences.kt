// Loads and saves application and account preferences for the mobile settings pages.
// The core owns proxy, signature, remote-image and alarm choices.
// The state refreshes the relevant values after a successful write.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountProxyParams
import jp.nonbili.meron.shared.AccountSignatureParams
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.AddPasswordAccountParams
import jp.nonbili.meron.shared.AppPrefsGetParams
import jp.nonbili.meron.shared.AppPrefsSetParams
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ProxyParams
import jp.nonbili.meron.shared.ProxySpec
import jp.nonbili.meron.shared.SignatureSpec
import jp.nonbili.meron.shared.encodeAppPrefValue
import jp.nonbili.meron.shared.parseAppPrefsResponse
import jp.nonbili.meron.shared.parseProxyResponse
import jp.nonbili.meron.shared.requireCoreOk
import jp.nonbili.meron.shared.sanitizeRemoteSenders
import jp.nonbili.meron.shared.untrustedCertificateProtocol
import jp.nonbili.meron.shared.withRemoteSender
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Read the app-wide proxy from the core store into [MeronMobileState.appProxy]. */
internal fun MeronMobileState.loadAppProxy(): Job? {
    if (!coreLoaded) return null
    val generation = ++proxyLoadGeneration
    return scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                MobileMailCommandClient(core).getProxy()
            }
        }.onSuccess {
            if (generation == proxyLoadGeneration) appProxy = parseProxyResponse(it)
        }
    }
}

/**
 * Persist the app-wide proxy. Live IMAP sessions keep their sockets; the change
 * lands as they reconnect, which is why the status line says so rather than
 * implying an instant switch.
 */
internal fun MeronMobileState.saveAppProxy(spec: ProxySpec) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val previous = appProxy
    appProxy = spec
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                MobileMailCommandClient(core).setProxy(ProxyParams(spec))
            }
        }.onSuccess {
            status = trs(if (spec.mode == "off") "mobile.status.proxyDisabled" else "mobile.status.proxySaved")
        }.onFailure {
            appProxy = previous
            status = trsError("accounts.toast.failedToUpdateProxy", it)
        }
    }
}

// How many times a failed app-signature read is retried, and how long between.
private const val APP_SIGNATURE_LOAD_ATTEMPTS = 3

private const val APP_SIGNATURE_RETRY_DELAY_MS = 250L

/**
 * Read the app-wide signature from the core store. It shares the desktop
 * `signature` row rather than a `mobile.*` one, so the two platforms agree after
 * a backup restore.
 */
internal fun MeronMobileState.loadAppSignature(): Job? {
    if (!coreLoaded) {
        // Nothing to read and nothing coming: callers waiting on this (the
        // `mailto:` handler) must not hang, the same as initialAccountsLoaded.
        appSignatureLoaded = true
        appSignatureLoadCompletion.complete(Unit)
        return null
    }
    // A reload (after a backup restore, say) makes the value on hand stale, so
    // compose waits again — and only the newest read may answer, or a slow
    // startup response could land on top of the restored signature.
    val generation = ++appSignatureLoadGeneration
    appSignatureLoaded = false
    appSignatureLoadCompletion.complete(Unit)
    appSignatureLoadCompletion = CompletableDeferred()
    return scope.launch {
        repeat(APP_SIGNATURE_LOAD_ATTEMPTS) { attempt ->
            val response =
                try {
                    withContext(ioDispatcher) {
                        // The core reports failure in the response body rather
                        // than by throwing, so validate before accepting it.
                        requireCoreOk(
                            MobileMailCommandClient(core).getPrefs(AppPrefsGetParams(listOf(APP_SIGNATURE_SETTING_KEY))),
                        )
                    }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (_: Throwable) {
                    null
                }
            if (generation != appSignatureLoadGeneration) return@launch
            if (response != null) {
                appSignatureHtml = parseAppPrefsResponse(response)[APP_SIGNATURE_SETTING_KEY] as? String ?: ""
                appSignatureLoaded = true
                appSignatureLoadCompletion.complete(Unit)
                return@launch
            }
            // A read can fail transiently on a cold start. Give up eventually so
            // a store that never opens cannot strand a `mailto:` link forever.
            if (attempt + 1 < APP_SIGNATURE_LOAD_ATTEMPTS) {
                delay(APP_SIGNATURE_RETRY_DELAY_MS * (attempt + 1))
            }
        }
        if (generation == appSignatureLoadGeneration) {
            appSignatureLoaded = true
            appSignatureLoadCompletion.complete(Unit)
        }
    }
}

/**
 * Read the app-wide remote-content sender allowlist from the core store. Like
 * the signature it shares the desktop row rather than a `mobile.*` one — and
 * unlike the signature nothing waits on it: a thread read before it lands shows
 * the account's own policy, and the reveal affordance with it.
 */
internal fun MeronMobileState.loadRemoteImageSenders(): Job? {
    if (!coreLoaded) return null
    val generation = ++remoteImageSendersLoadGeneration
    return scope.launch {
        runCatching { withContext(ioDispatcher) { readRemoteImageSenders() } }
            .onSuccess { stored ->
                // A write that landed while this read was in flight bumped the
                // generation: it already read the row itself, so this snapshot
                // is the older one and must not take the edit back off screen.
                if (generation == remoteImageSendersLoadGeneration) remoteImageSenders = stored
            }
    }
}

/** Read the default event alerts (timed and all-day) from the core store. */
internal fun MeronMobileState.loadAlertDefaults(): Job? {
    if (!coreLoaded) return null
    val generation = ++reminderLeadLoadGeneration
    return scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val response =
                    requireCoreOk(
                        MobileMailCommandClient(core).getPrefs(
                            AppPrefsGetParams(listOf(REMINDER_LEAD_SETTING_KEY, REMINDER_ALL_DAY_SETTING_KEY)),
                        ),
                    )
                parseAppPrefsResponse(response)
            }
        }.onSuccess { stored ->
            if (generation != reminderLeadLoadGeneration) return@onSuccess

            fun lead(
                key: String,
                allDay: Boolean,
            ): Int? = (stored[key] as? Number)?.toInt()?.takeIf { it == NO_ALERT || it in alertOptions(allDay) }
            reminderLeadMinutes = lead(REMINDER_LEAD_SETTING_KEY, false) ?: DEFAULT_ALERT_MINUTES
            reminderAllDayDays = lead(REMINDER_ALL_DAY_SETTING_KEY, true) ?: DEFAULT_ALL_DAY_ALERT_DAYS
        }
    }
}

/** Persist a default event alert; the core then has hosts reschedule. */
internal fun MeronMobileState.saveAlertDefault(
    allDay: Boolean,
    lead: Int,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }

    fun current() = if (allDay) reminderAllDayDays else reminderLeadMinutes

    fun show(value: Int) {
        if (allDay) reminderAllDayDays = value else reminderLeadMinutes = value
    }
    val previous = current()
    // Retire any read still in flight so it cannot take the edit back.
    ++reminderLeadLoadGeneration
    show(lead)
    val key = if (allDay) REMINDER_ALL_DAY_SETTING_KEY else REMINDER_LEAD_SETTING_KEY
    scope.launch {
        alertDefaultWrites.withLock {
            runCatching {
                withContext(ioDispatcher) {
                    requireCoreOk(MobileMailCommandClient(core).setPref(AppPrefsSetParams(key, encodeAppPrefValue(lead))))
                }
            }.onFailure {
                if (current() == lead) show(previous)
                status = trsError("mobile.status.reminderLeadUpdateFailed", it)
            }
        }
    }
}

/** The allowlist as the core store holds it, normalized. Runs on [ioDispatcher]. */
private suspend fun MeronMobileState.readRemoteImageSenders(): List<String> {
    val response =
        requireCoreOk(
            MobileMailCommandClient(core).getPrefs(AppPrefsGetParams(listOf(REMOTE_IMAGE_SENDERS_SETTING_KEY))),
        )
    val stored = parseAppPrefsResponse(response)[REMOTE_IMAGE_SENDERS_SETTING_KEY]
    return sanitizeRemoteSenders((stored as? List<*>).orEmpty().filterIsInstance<String>())
}

/**
 * Allow (or stop allowing) remote content from one sender.
 *
 * Unlike an account's "load remote images" toggle this is additive and app-wide: mail from [addr]
 * loads its remote content in every account.
 *
 * The core resolves the allowlist as it bakes a body, so messages already on screen are re-gated by
 * the reader rather than by a re-read of the thread.
 */
internal fun MeronMobileState.setRemoteImageSender(
    addr: String,
    allowed: Boolean,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val optimistic = withRemoteSender(remoteImageSenders, addr, allowed)
    if (optimistic == remoteImageSenders) return
    // Show the change at once; the write below decides what it really becomes.
    remoteImageSenders = optimistic
    scope.launch {
        // The row holds the whole list, so the edit is applied to what the store
        // actually has rather than to the snapshot this state happened to be
        // showing: a startup read that has not landed yet would otherwise write
        // an allowlist with every stored sender missing from it, and two edits
        // in quick succession would each drop the other's. The lock keeps the
        // read and the write it feeds one step, so those cases serialize.
        remoteImageSendersWrites.withLock {
            runCatching {
                withContext(ioDispatcher) {
                    val updated = withRemoteSender(readRemoteImageSenders(), addr, allowed)
                    requireCoreOk(
                        MobileMailCommandClient(core).setPref(
                            AppPrefsSetParams(REMOTE_IMAGE_SENDERS_SETTING_KEY, encodeAppPrefValue(updated)),
                        ),
                    )
                    updated
                }
            }.onSuccess { updated ->
                // Retire any read still in flight: this write knows the row.
                ++remoteImageSendersLoadGeneration
                remoteImageSenders = updated
            }.onFailure {
                // Undo this edit alone, against the list as it stands now — a
                // blanket restore of the pre-edit snapshot would take back the
                // edits that succeeded in between.
                remoteImageSenders = withRemoteSender(remoteImageSenders, addr, !allowed)
                status = trsError("mobile.status.remoteContentUpdateFailed", it)
            }
        }
    }
}

internal fun MeronMobileState.invalidateBackupReloads() {
    ++accountLoadGeneration
    ++proxyLoadGeneration
    ++appSignatureLoadGeneration
    ++remoteImageSendersLoadGeneration
    ++reminderLeadLoadGeneration
    accountsLoading = false
    appSignatureLoaded = false
    appSignatureLoadCompletion.complete(Unit)
    appSignatureLoadCompletion = CompletableDeferred()
}

internal suspend fun MeronMobileState.awaitAppSignatureLoaded() {
    while (!appSignatureLoaded) {
        appSignatureLoadCompletion.await()
    }
}

/** Persist the app-wide signature. Drafts already open keep what they carry. */
internal fun MeronMobileState.saveAppSignature(html: String) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val previous = appSignatureHtml
    appSignatureHtml = html
    reseedUntouchedQuickReply()
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                MobileMailCommandClient(core).setPref(AppPrefsSetParams(APP_SIGNATURE_SETTING_KEY, encodeAppPrefValue(html)))
            }
        }.onFailure {
            appSignatureHtml = previous
            reseedUntouchedQuickReply()
            status = trsError("mobile.status.signatureUpdateFailed", it)
        }
    }
}

/** Point one account at the app-wide signature, at none, or at its own. */
internal fun MeronMobileState.saveAccountSignature(
    account: AccountSummary,
    spec: SignatureSpec?,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                MobileMailCommandClient(core).setAccountSignature(AccountSignatureParams(account.id, spec))
            }
        }.onSuccess {
            // Joined, not fired and forgotten: the reseed below resolves the
            // signature off coreAccounts, which this refresh is what updates.
            listAccounts()?.join()
            reseedUntouchedQuickReply()
        }.onFailure {
            status = trsError("mobile.status.signatureUpdateFailed", it)
        }
    }
}

/** Point one account at its own proxy, at the app-wide one, or at none. */
internal fun MeronMobileState.saveAccountProxy(
    account: AccountSummary,
    spec: ProxySpec,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                MobileMailCommandClient(core).setAccountProxy(AccountProxyParams(account.id, spec))
            }
        }.onSuccess {
            listAccounts()
            status = trs("mobile.status.proxySaved")
        }.onFailure {
            status = trsError("accounts.toast.failedToUpdateProxy", it)
        }
    }
}

/**
 * Save an existing account's servers.
 *
 * The password is deliberately absent from the request unless the user typed a new one: the core reads
 * an omitted `password` as "keep the stored credential", so changing a port never costs the account
 * its login.
 *
 * A certificate the server cannot prove is offered for pinning here rather than dead-ending, exactly
 * as the sync and send paths do — but against the *edited* endpoint, since the save failed and the
 * stored account still names the old one.
 */
internal fun MeronMobileState.saveAccountServerSettings(
    account: AccountSummary,
    draft: ServerSettingsDraft,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val params =
        AddPasswordAccountParams(
            email = account.email,
            displayName = account.displayName,
            senderName = account.senderName,
            imapHost = draft.imapHost,
            imapPort = draft.imapPort,
            smtpHost = draft.smtpHost,
            smtpPort = draft.smtpPort,
            username = draft.username.ifBlank { account.email },
            password = draft.password,
            tls = draft.imapSecurity == MailSecurity.TLS,
            starttls = draft.imapSecurity == MailSecurity.STARTTLS,
            smtpTls = draft.smtpSecurity == MailSecurity.TLS,
            smtpStarttls = draft.smtpSecurity == MailSecurity.STARTTLS,
        )
    status = trs("mobile.status.savingServerSettings")
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                client.addPasswordAccount(params)
                client.listAccounts()
            }
        }.onSuccess {
            applyAccounts(it, preferEmail = account.email)
            errorBanner = null
            syncError = null
            status = trs("mobile.status.serverSettingsSaved")
            syncCoreThreads(accountOverride = account.id, folderOverride = INBOX_FOLDER, syncFirst = true)
        }.onFailure { failure ->
            val message = failure.message ?: trs("mobile.status.serverSettingsSaveFailed")
            if (untrustedCertificateProtocol(message) != null) {
                showTypedServerCertificate(
                    accountId = account.id,
                    imapHost = draft.imapHost,
                    imapPort = draft.imapPort,
                    imapSecurity = draft.imapSecurity,
                    smtpHost = draft.smtpHost,
                    smtpPort = draft.smtpPort,
                    smtpSecurity = draft.smtpSecurity,
                    proxy = account.proxy,
                    retry = PendingCertificateRetry.ServerSettings(account.id, draft),
                    message = message,
                )
            } else {
                errorBanner = message
                status = withReason(trs("mobile.status.serverSettingsSaveFailed"), failure.message) { key, args -> trs(key, args) }
            }
        }
    }
}
