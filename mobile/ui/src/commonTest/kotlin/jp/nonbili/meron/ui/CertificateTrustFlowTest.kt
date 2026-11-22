// A trust prompt can interrupt an in-flight send. The
// state tests check which refused operation resumes after
// trust is granted and which failures leave nothing to
// retry.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.CertificateProtocol
import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileCommand
import jp.nonbili.meron.shared.ProxySpec
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.isUntrustedCertificateError
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * A message refused by an untrusted submission certificate has to be *sent*
 * once the user trusts it — falling back to a sync would save the pin and leave
 * the message sitting unsent.
 */
class CertificateTrustFlowTest {
    @Test
    fun trustingTheCertificateSendsTheMessageThatWasRefused() =
        runBlocking {
            val core = TrustCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Ping"
            state.body = "Body${state.body}"

            state.sendMail()
            waitUntil { state.errorBanner != null }
            assertEquals(1, core.sendPayloads.size)
            assertTrue(isUntrustedCertificateError(assertNotNull(state.errorBanner)))

            state.showServerCertificate("a", assertNotNull(state.errorBanner), state.pendingCertificateRetry)
            waitUntil { state.certPrompt != null }
            val prompt = assertNotNull(state.certPrompt)
            assertEquals(CertificateProtocol.SMTP, prompt.protocol)
            assertEquals(1025, prompt.port)
            val probe = assertNotNull(core.probePayload)
            assertTrue(probe.contains("\"protocol\":\"smtp\""), probe)
            // Probed over the account's own proxy, like the send that failed.
            assertTrue(probe.contains("\"mode\":\"socks5\""), probe)

            core.sendFails = false
            state.trustPromptedCertificate()
            waitUntil { core.sendPayloads.size == 2 }
            val pin = assertNotNull(core.pinPayload)
            assertTrue(pin.contains("\"smtp_cert_pin\":\"6f69d6a7\""), pin)
            assertTrue(!pin.contains("\"cert_pin\":"), pin)
            // The same message, Message-ID included — not whatever the composer
            // holds by now.
            assertEquals(core.sendPayloads[0], core.sendPayloads[1])
            waitUntil { state.certPrompt == null && state.errorBanner == null }
            assertNull(state.pendingCertificateRetry)
        }

    @Test
    fun anOrdinarySendFailureLeavesNothingToResume() =
        runBlocking {
            val core = TrustCore(failure = "smtp auth: authentication failed")
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Ping"
            state.body = "Body${state.body}"

            state.sendMail()
            waitUntil { state.errorBanner != null }

            assertNull(state.pendingCertificateRetry)
        }

    @Test
    fun certificateAccountComesFromTheCrossAccountPendingSend() =
        runBlocking {
            val core = TrustCore()
            val state = state(core, this)
            state.coreAccounts = state.coreAccounts + account("b", smtpPort = 2025)
            state.selectedCoreAccountId = UNIFIED_ACCOUNT_ID
            state.openCompose()
            state.composeFromAccountId = "b"
            state.to = "you@example.com"
            state.subject = "Cross account"
            state.body = "Body"

            state.sendMail()
            waitUntil { state.errorBanner != null }

            assertEquals("b", certificateErrorAccountId("a", state.pendingCertificateRetry))
            state.showServerCertificate("a", assertNotNull(state.errorBanner), state.pendingCertificateRetry)
            waitUntil { state.certPrompt != null }
            assertEquals("b", state.certPrompt?.accountId)
            assertEquals(2025, state.certPrompt?.port)
        }

    @Test
    fun activePromptKeepsItsRetryWhenAnUnrelatedFailureIsRecorded() =
        runBlocking {
            val core = TrustCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Ping"
            state.body = "Body"
            state.sendMail()
            waitUntil { state.errorBanner != null }
            state.showServerCertificate("a", assertNotNull(state.errorBanner), state.pendingCertificateRetry)
            waitUntil { state.certPrompt != null }
            val promptRetry = assertNotNull(state.certPrompt).retry

            state.syncError = MobileSyncError("a", "network failed")
            state.errorBanner = null

            assertEquals(promptRetry, state.certPrompt?.retry)
            core.sendFails = false
            state.trustPromptedCertificate()
            waitUntil { core.sendPayloads.size == 2 }
            assertEquals(core.sendPayloads[0], core.sendPayloads[1])
        }

    @Test
    fun successfulTrustRetryDoesNotClearANewerComposeSession() =
        runBlocking {
            val core = TrustCore()
            val state = state(core, this)
            state.openCompose()
            state.to = "you@example.com"
            state.subject = "Old"
            state.body = "Old body"
            state.sendMail()
            waitUntil { state.errorBanner != null }
            state.showServerCertificate("a", assertNotNull(state.errorBanner), state.pendingCertificateRetry)
            waitUntil { state.certPrompt != null }

            state.openCompose()
            state.to = "new@example.com"
            state.subject = "New"
            state.body = "New body"
            core.sendFails = false
            state.trustPromptedCertificate()
            waitUntil { core.sendPayloads.size == 2 }

            assertEquals(Screen.Compose, state.screen)
            assertEquals("new@example.com", state.to)
            assertEquals("New", state.subject)
            assertEquals("New body", state.body)
        }

    @Test
    fun quickReplyTrustRetryReusesTheExactSendAndPreservesNewEditorText() =
        runBlocking {
            val core = TrustCore()
            val state = state(core, this)
            state.selectedCoreThread =
                ThreadSummary(id = "t1", accountId = "a", folder = "INBOX", subject = "Ping", sender = "sender@example.com")
            state.quickReplyThreadId = "t1"
            state.messages =
                listOf(
                    MessageBody(
                        id = "m1",
                        folderId = "INBOX",
                        from = "Sender",
                        fromAddr = "sender@example.com",
                        to = "a@example.com",
                        subject = "Ping",
                        body = "Original",
                        messageId = "original@example.com",
                    ),
                )
            state.onQuickReplyBodyChange("First reply")

            state.sendQuickReply()
            waitUntil { state.errorBanner != null }
            val firstPayload = core.sendPayloads.single()
            val bubbleId = assertNotNull(state.pendingQuickReplySend).tempMessageId
            state.onQuickReplyBodyChange("Changed while prompting")
            state.showServerCertificate("a", assertNotNull(state.errorBanner), state.pendingCertificateRetry)
            waitUntil { state.certPrompt != null }
            core.sendFails = false
            state.trustPromptedCertificate()
            waitUntil { core.sendPayloads.size == 2 }

            assertEquals(firstPayload, core.sendPayloads[1])
            assertEquals(1, core.identityAllocations)
            assertEquals(1, state.messages.count { it.id == bubbleId })
            assertEquals("Changed while prompting", state.quickReplyBody)
        }

    @Test
    fun quickReplyTrustRetryKeepsADraftReusedByNewerEdits() =
        runBlocking {
            val core = TrustCore()
            val state = state(core, this)
            state.selectedCoreThread =
                ThreadSummary(id = "t1", accountId = "a", folder = "INBOX", subject = "Ping", sender = "sender@example.com")
            state.quickReplyThreadId = "t1"
            state.messages =
                listOf(
                    MessageBody(
                        id = "m1",
                        folderId = "INBOX",
                        from = "Sender",
                        fromAddr = "sender@example.com",
                        to = "a@example.com",
                        subject = "Ping",
                        body = "Original",
                        messageId = "original@example.com",
                    ),
                )
            state.quickReplyBody = "First reply"
            state.quickReplyDraftId = "reply-draft@example.com"
            state.quickReplyDraftSaved = true

            state.sendQuickReply()
            waitUntil { state.errorBanner != null }
            state.onQuickReplyBodyChange("Newer saved reply")
            state.autoSaveQuickReplyDraft()
            waitUntil { core.savedDraftPayloads.isNotEmpty() }
            state.showServerCertificate("a", assertNotNull(state.errorBanner), state.pendingCertificateRetry)
            waitUntil { state.certPrompt != null }
            core.sendFails = false
            state.trustPromptedCertificate()
            waitUntil { core.sendPayloads.size == 2 }

            assertTrue(core.discardDraftPayloads.isEmpty())
            assertEquals("reply-draft@example.com", state.quickReplyDraftId)
            assertEquals(true, state.quickReplyDraftSaved)
            assertEquals("Newer saved reply", state.quickReplyBody)
        }

    @Test
    fun freshQuickReplyPreparationDropsAnOlderRetryWhenIdentityAllocationFails() =
        runBlocking {
            val core = TrustCore()
            val state = state(core, this)
            state.selectedCoreThread =
                ThreadSummary(id = "t1", accountId = "a", folder = "INBOX", subject = "Ping", sender = "sender@example.com")
            state.quickReplyThreadId = "t1"
            state.messages =
                listOf(
                    MessageBody(
                        id = "m1",
                        folderId = "INBOX",
                        from = "Sender",
                        fromAddr = "sender@example.com",
                        to = "a@example.com",
                        subject = "Ping",
                        body = "Original",
                        messageId = "original@example.com",
                    ),
                )
            state.onQuickReplyBodyChange("Old reply")
            state.sendQuickReply()
            waitUntil { state.pendingQuickReplySend != null && !state.quickReplySendInFlight }
            assertEquals(1, core.sendPayloads.size)

            state.onQuickReplyBodyChange("New reply")
            core.failIdentityAllocation = true
            state.sendQuickReply()
            waitUntil { !state.quickReplySendInFlight && state.quickReplyFailure.contains("allocation failed") }

            assertNull(state.pendingQuickReplySend)
            state.retryQuickReplySend()
            delay(20)
            assertEquals(1, core.sendPayloads.size)
        }

    @Test
    fun quickReplyFailureAfterNavigationDoesNotExposeTheOldRetry() =
        runBlocking {
            val core = TrustCore().apply { sendGate = CompletableDeferred() }
            val state = state(core, this)
            state.selectedCoreThread =
                ThreadSummary(id = "old", accountId = "a", folder = "INBOX", subject = "Old", sender = "old@example.com")
            state.quickReplyThreadId = "old"
            state.messages =
                listOf(
                    MessageBody(
                        id = "old-message",
                        folderId = "INBOX",
                        from = "Old",
                        fromAddr = "old@example.com",
                        to = "a@example.com",
                        subject = "Old",
                        body = "Original",
                        messageId = "old@example.com",
                    ),
                )
            state.onQuickReplyBodyChange("Old reply")
            state.sendQuickReply()
            waitUntil { core.sendPayloads.size == 1 }

            state.selectedCoreThread =
                ThreadSummary(id = "new", accountId = "a", folder = "INBOX", subject = "New", sender = "new@example.com")
            state.quickReplyThreadId = "new"
            state.quickReplyBody = "New reply"
            state.quickReplyFailure = ""
            state.messages = emptyList()
            ++state.quickReplyGeneration
            assertNotNull(core.sendGate).complete(Unit)
            waitUntil { !state.quickReplySendInFlight }

            assertEquals("", state.quickReplyFailure)
            state.retryQuickReplySend()
            delay(20)
            assertEquals(1, core.sendPayloads.size)
        }

    private suspend fun waitUntil(condition: () -> Boolean) {
        withTimeout(5_000) {
            while (!condition()) delay(5)
        }
    }

    private fun state(
        core: MeronCore,
        scope: CoroutineScope,
    ): MeronMobileState =
        MeronMobileState(
            scope = scope,
            core = core,
            coreLoaded = true,
            prefs = MemoryPreferences(),
            kanbanPrefs = MemoryPreferences(),
            services = NoopPlatformServices(),
            locale = NoopLocaleController(),
            mobileHost = DefaultMobileHost(),
            settingsMirror = SettingsMirror(core, MemoryPreferences()) { true },
        ).apply {
            coreAccounts =
                listOf(
                    account("a"),
                )
            selectedCoreAccountId = "a"
            appSignatureLoaded = true
        }

    private fun account(
        id: String,
        smtpPort: Int = 1025,
    ): AccountSummary =
        AccountSummary(
            id = id,
            email = "$id@example.com",
            imapHost = "127.0.0.1",
            imapPort = 1143,
            smtpHost = "127.0.0.1",
            smtpPort = smtpPort,
            tls = false,
            starttls = true,
            smtpTls = false,
            smtpStarttls = true,
            proxy = ProxySpec("socks5", "127.0.0.1", 9050),
        )

    /** Refuses the first send the way a bridge with a self-signed certificate does. */
    private class TrustCore(
        private val failure: String =
            "smtp-untrusted-certificate: invalid peer certificate: Other(OtherError(CaUsedAsEndEntity))",
    ) : MeronCore {
        val sendPayloads = mutableListOf<String>()
        val savedDraftPayloads = mutableListOf<String>()
        val discardDraftPayloads = mutableListOf<String>()
        var probePayload: String? = null
        var pinPayload: String? = null
        var sendFails = true
        var identityAllocations = 0
        var failIdentityAllocation = false
        var sendGate: CompletableDeferred<Unit>? = null

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                MobileCommand.AllocateIdentity -> {
                    identityAllocations += 1
                    if (failIdentityAllocation) throw RuntimeException("allocation failed")
                    """{"message_id":"draft-1@example.com"}"""
                }

                MobileCommand.Send -> {
                    sendPayloads += payloadJson
                    sendGate?.await()
                    if (sendFails) throw RuntimeException(failure)
                    "{}"
                }

                MobileCommand.SaveDraft -> {
                    savedDraftPayloads += payloadJson
                    "{}"
                }

                MobileCommand.DiscardDraft -> {
                    discardDraftPayloads += payloadJson
                    "{}"
                }

                MobileCommand.AccountProbeCert -> {
                    probePayload = payloadJson
                    """
                    {"certificate":{"fingerprint":"6f69d6a7","subject":"CN=127.0.0.1, O=Proton Mail Bridge",
                    "issuer":"CN=127.0.0.1, O=Proton Mail Bridge","not_before":"Sat, 22 Aug 2026 23:57:04 +0000",
                    "not_after":"Mon, 24 Aug 2026 23:57:04 +0000","self_signed":true}}
                    """.trimIndent()
                }

                MobileCommand.AccountSetCertPin -> {
                    pinPayload = payloadJson
                    """{"ok":true}"""
                }

                else -> {
                    "{}"
                }
            }

        override fun events(): CoreEventStream =
            object : CoreEventStream {
                override fun subscribe(listener: (CoreEvent) -> Unit): CloseableHandle = CloseableHandle {}
            }

        override suspend fun protocolVersion(): Int = 0
    }

    private class MemoryPreferences : AppPreferences {
        private val values = mutableMapOf<String, String>()

        override fun getString(
            key: String,
            default: String,
        ): String = values[key] ?: default

        override fun putString(
            key: String,
            value: String,
        ) {
            values[key] = value
        }

        override fun getBoolean(
            key: String,
            default: Boolean,
        ): Boolean = default

        override fun putBoolean(
            key: String,
            value: Boolean,
        ) {}

        override fun getInt(
            key: String,
            default: Int,
        ): Int = default

        override fun putInt(
            key: String,
            value: Int,
        ) {}

        override fun getStringSet(
            key: String,
            default: Set<String>,
        ): Set<String> = default

        override fun putStringSet(
            key: String,
            value: Set<String>,
        ) {}

        override fun remove(key: String) {
            values.remove(key)
        }
    }

    private class NoopPlatformServices : PlatformServices {
        override fun openUrl(url: String) {}

        override fun openOAuthUrl(
            url: String,
            callbackScheme: String,
            onCallback: (String) -> Unit,
            onFailure: (String) -> Unit,
        ) {}

        override fun copyText(
            label: String,
            value: String,
        ) {}

        override fun copyImage(
            bytes: ByteArray,
            mimeType: String,
            label: String,
        ) {}

        override fun shareFile(
            bytes: ByteArray,
            fileName: String,
            mimeType: String,
        ) {}

        override fun saveFile(
            bytes: ByteArray,
            fileName: String,
            mimeType: String,
        ) {}

        override fun pickFile(
            mimeTypes: List<String>,
            onPicked: (PickedFile?) -> Unit,
        ) {}

        override fun pickImage(onPicked: (PickedFile?) -> Unit) {}
    }

    private class NoopLocaleController : LocaleController {
        override fun systemLanguageTag(): String = ""

        override fun applySystem(tag: String) {}

        override fun deviceLanguageTag(): String = "en-US"

        override fun displayName(tag: String): String = tag
    }
}
