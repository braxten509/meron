// Defines the shared account, proxy and sending-identity models.
// The mobile protocol fills these values from core account responses.
// Compose and account settings use the same identities when choosing a From address.

package jp.nonbili.meron.shared

data class AccountSummary(
    val id: String,
    val email: String,
    val displayName: String = "",
    val senderName: String = "",
    val avatarUrl: String = "",
    val needsReconnect: Boolean = false,
    val engine: String = "",
    val provider: String = "",
    val authType: String = "",
    /** The IMAP/SMTP login, which is not always the address. */
    val username: String = "",
    val imapHost: String = "",
    val imapPort: Int = 0,
    val smtpHost: String = "",
    val smtpPort: Int = 0,
    val tls: Boolean = true,
    val starttls: Boolean = false,
    val smtpTls: Boolean = true,
    val smtpStarttls: Boolean = false,
    val loadRemoteImages: Boolean = false,
    val includedInUnified: Boolean = true,
    val muted: Boolean = false,
    val paused: Boolean = false,
    val conversationHtml: Boolean = true,
    val saveSentCopy: Boolean? = null,
    val rssSyncIntervalMinutes: Int = 60,
    val aliases: List<AccountAlias> = emptyList(),
    val chatWallpaperKind: String = "",
    val chatWallpaperPresetId: String = "",
    val chatWallpaperUrl: String = "",
    val proxy: ProxySpec = ProxySpec.followApp,
    val signature: SignatureSpec = SignatureSpec.followApp,
)

/**
 * An account's signature choice. [mode] is "global" (follow the app-wide
 * signature, the default), "none" (send no signature) or "custom" (use [html]).
 * The html is kept across mode changes so switching away and back does not lose
 * what the user wrote.
 */
data class SignatureSpec(
    val mode: String = "global",
    val html: String = "",
) {
    companion object {
        val followApp = SignatureSpec()
    }
}

/**
 * A proxy endpoint, used for both the app-wide setting and a per-account
 * override. [mode] is "off"/"http"/"socks5" app-wide, and adds "global" (follow
 * the app proxy, the default) and "direct" (never proxy) per account. An empty
 * [username] means the proxy needs no authentication.
 */
data class ProxySpec(
    val mode: String = "off",
    val host: String = "",
    val port: Int = 0,
    val username: String = "",
    val password: String = "",
) {
    /** Whether this is filled in well enough for the core to actually use it. */
    val usable: Boolean
        get() = (mode == "http" || mode == "socks5") && host.isNotBlank() && port > 0

    companion object {
        val off = ProxySpec()
        val followApp = ProxySpec(mode = "global")
    }
}

data class AccountAlias(
    val email: String,
    val name: String = "",
)

data class ContactSuggestion(
    val name: String,
    val addr: String,
)

data class SendIdentity(
    val accountId: String,
    val email: String,
    val name: String = "",
)
