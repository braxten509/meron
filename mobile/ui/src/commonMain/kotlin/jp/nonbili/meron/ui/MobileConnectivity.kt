// Represents mobile account connectivity and builds
// the labels shown for failures. Accounts with
// similar display names can require extra identifying
// detail.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.AddPasswordAccountParams
import jp.nonbili.meron.shared.CertificateProtocol
import jp.nonbili.meron.shared.SendMailParams
import jp.nonbili.meron.shared.ServerCertificate
import kotlinx.coroutines.CancellationException

internal data class MobileSyncError(
    val accountId: String?,
    val message: String,
)

/**
 * A server certificate the user is being asked to trust, and the account and
 * server it belongs to. [protocol] decides which pin an acceptance writes.
 */
internal data class MobileCertPrompt(
    val accountId: String,
    val host: String,
    val port: Int,
    val protocol: CertificateProtocol,
    val certificate: ServerCertificate,
    val retry: PendingCertificateRetry?,
)

/** A prepared message whose send failed and can be retried as-is. */
internal data class PendingComposeSend(
    val accountId: String,
    val params: SendMailParams,
    val composeSessionGeneration: Int,
    val draftOwners: List<ComposeDraftOwner>,
)

/** An immutable quick reply, including the optimistic bubble it owns. */
internal data class PendingQuickReplySend(
    val accountId: String,
    val params: SendMailParams,
    val tempMessageId: String,
    val threadId: String,
    val draftOwner: ComposeDraftOwner?,
    val quickReplyGeneration: Int,
)

internal sealed interface PendingCertificateRetry {
    val accountId: String

    data class Compose(
        val pending: PendingComposeSend,
    ) : PendingCertificateRetry {
        override val accountId: String = pending.accountId
    }

    data class QuickReply(
        val pending: PendingQuickReplySend,
    ) : PendingCertificateRetry {
        override val accountId: String = pending.accountId
    }

    /**
     * A server-settings save a certificate refusal interrupted. It carries the
     * edited servers because they are *not* stored yet — the save failed — so
     * both the probe and the retry have to use what the user typed rather than
     * the account's current values.
     */
    data class ServerSettings(
        override val accountId: String,
        val draft: ServerSettingsDraft,
    ) : PendingCertificateRetry

    /**
     * An account creation a certificate refusal interrupted. Unlike every other
     * case there is no stored account yet, so the accepted pin cannot be written
     * to a row and read back — it has to ride along on the request that retries.
     * [accountId] is the id the core will mint for this address.
     */
    data class AddAccount(
        override val accountId: String,
        val params: AddPasswordAccountParams,
    ) : PendingCertificateRetry
}

internal fun certificateErrorAccountId(
    selectedAccountId: String?,
    retry: PendingCertificateRetry?,
): String? = retry?.accountId ?: selectedAccountId

internal class AccountSyncException(
    val accountId: String,
    cause: Throwable,
) : Exception(cause.message, cause)

internal suspend fun <T> withSyncAccountContext(
    accountId: String,
    block: suspend () -> T,
): T =
    try {
        block()
    } catch (failure: CancellationException) {
        throw failure
    } catch (failure: Throwable) {
        throw AccountSyncException(accountId, failure)
    }

internal fun mobileConnectivityAccountLabel(
    accountId: String?,
    accounts: List<AccountSummary>,
): String? {
    if (accountId.isNullOrBlank()) return null
    val account = accounts.firstOrNull { it.id == accountId } ?: return accountId
    val displayName = account.displayName.trim()
    val email = account.email.trim()
    if (displayName.isBlank()) return email.ifBlank { account.id }

    val duplicateName =
        accounts.any { candidate ->
            candidate.id != account.id &&
                candidate.displayName.trim().equals(displayName, ignoreCase = true)
        }
    return if (duplicateName && email.isNotBlank() && !email.equals(displayName, ignoreCase = true)) {
        "$displayName ($email)"
    } else {
        displayName
    }
}

internal fun mobileProxyEndpointFromSyncError(error: String): String? {
    val endpoint = """([^\s]+:\d+)"""
    return Regex("""\bconnect to proxy\s+$endpoint""", RegexOption.IGNORE_CASE).find(error)?.groupValues?.get(1)
        ?: Regex("""\b(?:http|socks5) proxy\s+$endpoint""", RegexOption.IGNORE_CASE).find(error)?.groupValues?.get(1)
}
