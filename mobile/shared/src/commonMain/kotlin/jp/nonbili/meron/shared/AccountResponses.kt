// Account discovery, connection and profile data returned by the shared core.
// Account list and server probe results supply the mobile settings screens.
// The public parsing entry points delegate here from MobileResponseParsers.
package jp.nonbili.meron.shared

internal fun decodeAccountListResponse(responseJson: String): List<AccountSummary> {
    val accountsJson = responseJson.findJsonArrayProperty("accounts") ?: return emptyList()
    return accountsJson.jsonArrayElements().mapNotNull { item ->
        val id = item.findJsonStringProperty("id").orEmpty()
        if (id.isBlank()) return@mapNotNull null
        val wallpaperJson = item.findJsonPropertyValue("chat_wallpaper").orEmpty()
        AccountSummary(
            id = id,
            email = item.findJsonStringProperty("email").orEmpty(),
            displayName = item.findJsonStringProperty("display_name").orEmpty(),
            senderName = item.findJsonStringProperty("sender_name").orEmpty(),
            avatarUrl = item.findJsonStringProperty("avatar_url").orEmpty(),
            needsReconnect = item.findJsonBooleanProperty("needs_reconnect") ?: false,
            engine = item.findJsonStringProperty("engine").orEmpty(),
            provider = item.findJsonStringProperty("provider").orEmpty(),
            authType = item.findJsonStringProperty("auth_type").orEmpty(),
            username = item.findJsonStringProperty("username").orEmpty(),
            imapHost = item.findJsonStringProperty("imap_host").orEmpty(),
            imapPort = item.findJsonLongProperty("imap_port")?.toInt() ?: 0,
            smtpHost = item.findJsonStringProperty("smtp_host").orEmpty(),
            smtpPort = item.findJsonLongProperty("smtp_port")?.toInt() ?: 0,
            tls = item.findJsonBooleanProperty("tls") ?: true,
            starttls = item.findJsonBooleanProperty("starttls") ?: false,
            smtpTls = item.findJsonBooleanProperty("smtp_tls") ?: true,
            smtpStarttls = item.findJsonBooleanProperty("smtp_starttls") ?: false,
            loadRemoteImages = item.findJsonBooleanProperty("load_remote_images") ?: false,
            includedInUnified = item.findJsonBooleanProperty("included_in_unified") ?: true,
            muted = item.findJsonBooleanProperty("muted") ?: false,
            paused = item.findJsonBooleanProperty("paused") ?: false,
            conversationHtml = item.findJsonBooleanProperty("conversation_html") ?: true,
            saveSentCopy = item.findJsonBooleanProperty("save_sent_copy"),
            rssSyncIntervalMinutes = item.findJsonLongProperty("rss_sync_interval_minutes")?.toInt() ?: 60,
            aliases =
                item
                    .findJsonArrayProperty("aliases")
                    ?.jsonArrayElements()
                    ?.mapNotNull { aliasJson ->
                        val email = aliasJson.findJsonStringProperty("email").orEmpty()
                        if (email.isBlank()) {
                            null
                        } else {
                            AccountAlias(
                                email = email,
                                name = aliasJson.findJsonStringProperty("name").orEmpty(),
                            )
                        }
                    }.orEmpty(),
            proxy = parseProxySpec(item.findJsonPropertyValue("proxy"), ProxySpec.followApp),
            signature = parseSignatureSpec(item.findJsonPropertyValue("signature")),
            chatWallpaperKind = wallpaperJson.findJsonStringProperty("kind").orEmpty(),
            chatWallpaperPresetId = wallpaperJson.findJsonStringProperty("presetId").orEmpty(),
            chatWallpaperUrl = wallpaperJson.findJsonStringProperty("url").orEmpty(),
        )
    }
}

/**
 * Read an account's signature override. Absent (or a mode this build does not
 * know) means the account follows the app-wide signature.
 */
private fun parseSignatureSpec(signatureJson: String?): SignatureSpec {
    val json = signatureJson?.takeIf { it.isNotBlank() && it != "null" } ?: return SignatureSpec.followApp
    val mode = json.findJsonStringProperty("mode").orEmpty()
    if (mode != "global" && mode != "none" && mode != "custom") return SignatureSpec.followApp
    return SignatureSpec(mode = mode, html = json.findJsonStringProperty("html").orEmpty())
}

/**
 * Read a proxy object, falling back to [fallback] when it is absent or carries
 * a mode this build does not know. Accounts stored before proxy support have no
 * such object at all, which is why the fallback is the caller's business.
 */
internal fun parseProxySpec(
    proxyJson: String?,
    fallback: ProxySpec,
): ProxySpec {
    val json = proxyJson?.takeIf { it.isNotBlank() && it != "null" } ?: return fallback
    val mode = json.findJsonStringProperty("mode").orEmpty()
    if (mode.isBlank()) return fallback
    return ProxySpec(
        mode = mode,
        host = json.findJsonStringProperty("host").orEmpty(),
        port = json.findJsonLongProperty("port")?.toInt() ?: 0,
        username = json.findJsonStringProperty("username").orEmpty(),
        password = json.findJsonStringProperty("password").orEmpty(),
    )
}

internal fun decodeContactSuggestResponse(responseJson: String): List<ContactSuggestion> {
    val contactsJson = responseJson.findJsonArrayProperty("contacts") ?: return emptyList()
    return contactsJson.jsonArrayElements().mapNotNull { item ->
        val addr = item.findJsonStringProperty("addr").orEmpty()
        if (addr.isBlank()) return@mapNotNull null
        ContactSuggestion(
            name = item.findJsonStringProperty("name").orEmpty(),
            addr = addr,
        )
    }
}

data class DiscoveredAccountSettings(
    val imapHost: String,
    val imapPort: Int,
    val smtpHost: String,
    val smtpPort: Int,
    val username: String,
    val providerName: String,
    val source: String,
    val appPasswordProvider: String,
    val appPasswordUrl: String,
)

internal fun decodeAutodiscoverResponse(responseJson: String): DiscoveredAccountSettings {
    val hintJson = responseJson.findJsonPropertyValue("app_password_hint").orEmpty()
    return DiscoveredAccountSettings(
        imapHost = responseJson.findJsonStringProperty("imap_host").orEmpty(),
        imapPort = responseJson.findJsonLongProperty("imap_port")?.toInt() ?: 993,
        smtpHost = responseJson.findJsonStringProperty("smtp_host").orEmpty(),
        smtpPort = responseJson.findJsonLongProperty("smtp_port")?.toInt() ?: 465,
        username = responseJson.findJsonStringProperty("username").orEmpty(),
        providerName = responseJson.findJsonStringProperty("provider_name").orEmpty(),
        source = responseJson.findJsonStringProperty("source").orEmpty(),
        appPasswordProvider = hintJson.findJsonStringProperty("provider").orEmpty(),
        appPasswordUrl = hintJson.findJsonStringProperty("url").orEmpty(),
    )
}

/**
 * A server certificate the user is asked to trust. [fingerprint] is the hex
 * SHA-256 of the certificate, and the value pinned when they accept.
 */
data class ServerCertificate(
    val fingerprint: String,
    val subject: String,
    val issuer: String,
    val notBefore: String,
    val notAfter: String,
    val selfSigned: Boolean,
)

internal fun decodeProbeCertResponse(responseJson: String): ServerCertificate? {
    val certJson = responseJson.findJsonObjectProperty("certificate") ?: return null
    val fingerprint = certJson.findJsonStringProperty("fingerprint").orEmpty()
    if (fingerprint.isBlank()) return null
    return ServerCertificate(
        fingerprint = fingerprint,
        subject = certJson.findJsonStringProperty("subject").orEmpty(),
        issuer = certJson.findJsonStringProperty("issuer").orEmpty(),
        notBefore = certJson.findJsonStringProperty("not_before").orEmpty(),
        notAfter = certJson.findJsonStringProperty("not_after").orEmpty(),
        selfSigned = certJson.findJsonBooleanProperty("self_signed") ?: false,
    )
}
