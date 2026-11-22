// Compose lifecycle fixture.
// The mobile host suites exercise these cases with deterministic core responses.
// The tests run without a device or a live mail account.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileCommand
import jp.nonbili.meron.shared.SignatureSpec
import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.yield

internal abstract class ComposeLifecycleFixture {
    /**
     * Wait for composer state the core call writes *after* it returns. The `CompletableDeferred`s in
     * [SaveCore] are completed inside the call, which runs on [ioDispatcher], so awaiting one can resume
     * this test before the caller's `withContext` return is even queued on the `runBlocking` loop — a
     * single `yield()` is not a barrier across that.
     */
    protected suspend fun awaitState(settled: () -> Boolean) {
        withTimeout(1_000) {
            while (!settled()) yield()
        }
    }

    protected fun state(
        core: MeronCore,
        scope: CoroutineScope,
    ): MeronMobileState =
        MeronMobileState(
            scope = scope,
            core = core,
            coreLoaded = true,
            prefs = TestPreferences(),
            kanbanPrefs = TestPreferences(),
            services = TestPlatformServices(),
            locale = TestLocaleController(),
            mobileHost = DefaultMobileHost(),
            settingsMirror = SettingsMirror(core, TestPreferences()) { true },
        ).apply {
            coreAccounts =
                listOf(
                    AccountSummary(
                        id = "a",
                        email = "a@example.com",
                        signature = SignatureSpec("custom", "<p>From A</p>"),
                    ),
                    AccountSummary(
                        id = "b",
                        email = "b@example.com",
                        signature = SignatureSpec("custom", "<p>From B</p>"),
                    ),
                )
            selectedCoreAccountId = "a"
            appSignatureLoaded = true
        }

    /** The server copy of the quick reply just sent, still sitting in Drafts. */

    protected fun consumedDraft(): MessageBody =
        MessageBody(
            id = "draft-row",
            folderId = "Drafts",
            from = "A",
            fromAddr = "a@example.com",
            to = "sender@example.com",
            subject = "Re: Subject",
            body = "Reply",
            messageId = "reply-draft@example.com",
        )

    protected fun threadReadWithConsumedDraft(messageId: String = "reply-draft@example.com"): String = """{"messages":[{"id":"message","folder_id":"INBOX","from_addr":"sender@example.com","to":"a@example.com","subject":"Subject","body":"Original","message_id":"original@example.com"},{"id":"draft-row","folder_id":"Drafts","from_addr":"a@example.com","to":"sender@example.com","subject":"Re: Subject","body":"Reply","message_id":"$messageId"}]}"""

    protected fun prepareQuickReply(
        state: MeronMobileState,
        rowId: String = "thread",
        backendThreadId: String = "",
    ) {
        state.selectedCoreThread =
            ThreadSummary(
                id = rowId,
                accountId = "a",
                folder = "INBOX",
                subject = "Subject",
                sender = "sender@example.com",
                threadId = backendThreadId,
            )
        state.quickReplyThreadId = backendThreadId.ifBlank { rowId }
        state.messages =
            listOf(
                MessageBody(
                    id = "message",
                    folderId = "INBOX",
                    from = "Sender",
                    fromAddr = "sender@example.com",
                    to = "a@example.com",
                    subject = "Subject",
                    body = "Original",
                    messageId = "original@example.com",
                ),
            )
    }

    protected class SaveCore : MeronCore {
        val firstSaveStarted = CompletableDeferred<Unit>()
        val firstSaveFinished = CompletableDeferred<Unit>()
        val releaseFirstSave = CompletableDeferred<Unit>()
        val secondSaveFinished = CompletableDeferred<Unit>()
        val sendFinished = CompletableDeferred<Unit>()
        val discardFinished = CompletableDeferred<Unit>()
        val savedPayloads = mutableListOf<String>()
        val discardPayloads = mutableListOf<String>()
        var saveCalls = 0
        var sendCalls = 0
        var discardFails = false

        // Fails this many discards before the rest succeed.
        var discardFailuresLeft = 0
        var allocationFails = false
        var sendFails = false
        var holdDiscard: CompletableDeferred<Unit>? = null

        // Set by tests that need the send itself to stay in flight while they act.
        var holdSend: CompletableDeferred<Unit>? = null
        var allocationCalls = 0
        var threadReadCalls = 0
        var threadReadResponse = "{}"

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                MobileCommand.AllocateIdentity -> {
                    allocationCalls++
                    if (allocationFails) throw RuntimeException("allocation failed")
                    """{"message_id":"draft-$allocationCalls@example.com"}"""
                }

                MobileCommand.SaveDraft -> {
                    savedPayloads += payloadJson
                    saveCalls++
                    if (saveCalls == 1) {
                        firstSaveStarted.complete(Unit)
                        releaseFirstSave.await()
                        firstSaveFinished.complete(Unit)
                    } else {
                        secondSaveFinished.complete(Unit)
                    }
                    "{}"
                }

                MobileCommand.DiscardDraft -> {
                    discardPayloads += payloadJson
                    discardFinished.complete(Unit)
                    // Set by tests that need the post-send discard to stay in
                    // flight while they act; nothing to wait on otherwise.
                    holdDiscard?.await()
                    if (discardFails || discardFailuresLeft-- > 0) throw RuntimeException("discard failed")
                    "{}"
                }

                MobileCommand.Send -> {
                    sendCalls++
                    sendFinished.complete(Unit)
                    holdSend?.await()
                    if (sendFails) throw RuntimeException("send failed")
                    "{}"
                }

                MobileCommand.ThreadRead -> {
                    threadReadCalls++
                    threadReadResponse
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

    protected class ReloadCore : MeronCore {
        val oldAccountsStarted = CompletableDeferred<Unit>()
        val newAccountsStarted = CompletableDeferred<Unit>()
        val oldProxyStarted = CompletableDeferred<Unit>()
        val newProxyStarted = CompletableDeferred<Unit>()
        val oldAccounts = CompletableDeferred<String>()
        val newAccounts = CompletableDeferred<String>()
        val oldProxy = CompletableDeferred<String>()
        val newProxy = CompletableDeferred<String>()
        private var accountCalls = 0
        private var proxyCalls = 0

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                MobileCommand.AccountList -> {
                    accountCalls++
                    if (accountCalls == 1) {
                        oldAccountsStarted.complete(Unit)
                        oldAccounts.await()
                    } else {
                        newAccountsStarted.complete(Unit)
                        newAccounts.await()
                    }
                }

                MobileCommand.AppProxyGet -> {
                    proxyCalls++
                    if (proxyCalls == 1) {
                        oldProxyStarted.complete(Unit)
                        oldProxy.await()
                    } else {
                        newProxyStarted.complete(Unit)
                        newProxy.await()
                    }
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

    protected class TestPreferences : AppPreferences {
        private val strings = mutableMapOf<String, String>()

        override fun getString(
            key: String,
            default: String,
        ): String = strings[key] ?: default

        override fun putString(
            key: String,
            value: String,
        ) {
            strings[key] = value
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
            strings.remove(key)
        }
    }

    protected class TestPlatformServices : PlatformServices {
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

    protected class TestLocaleController : LocaleController {
        override fun systemLanguageTag(): String = ""

        override fun applySystem(tag: String) {}

        override fun deviceLanguageTag(): String = "en-US"

        override fun displayName(tag: String): String = tag
    }
}
