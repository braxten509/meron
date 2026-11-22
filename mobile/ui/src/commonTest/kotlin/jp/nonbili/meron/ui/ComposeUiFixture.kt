// Compose ui fixture.
// The mobile host suites exercise these cases with deterministic core responses.
// The tests run without a device or a live mail account.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountAlias
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.CoroutineScope
import kotlin.coroutines.EmptyCoroutineContext

internal abstract class ComposeUiFixture {
    protected fun aliasAccount(aliases: List<AccountAlias> = listOf(AccountAlias(email = "sales@example.com", name = "Sales"))): AccountSummary =
        AccountSummary(
            id = "acc",
            email = "me@example.com",
            senderName = "Me",
            aliases = aliases,
        )

    protected fun threadSummary(
        id: String,
        threadId: String = "",
    ): ThreadSummary =
        ThreadSummary(
            id = id,
            accountId = "acc",
            folder = "INBOX",
            subject = "Subject",
            sender = "sender@example.com",
            threadId = threadId,
        )

    protected fun messageBody(
        id: String,
        folderId: String,
        inReplyTo: String = "",
        references: String = "",
        messageId: String = "",
        to: String = "me@example.com",
        fromAddr: String = "",
        outgoing: Boolean = false,
    ): MessageBody =
        MessageBody(
            id = id,
            folderId = folderId,
            from = "sender@example.com",
            to = to,
            subject = "Subject",
            body = "Body",
            inReplyTo = inReplyTo,
            references = references,
            messageId = messageId,
            fromAddr = fromAddr,
            outgoing = outgoing,
        )

    protected fun testState(): MeronMobileState =
        MeronMobileState(
            scope = CoroutineScope(EmptyCoroutineContext),
            core = FakeCore(),
            coreLoaded = true,
            prefs = FakePreferences(),
            kanbanPrefs = FakePreferences(),
            services = FakePlatformServices(),
            locale = FakeLocaleController(),
            mobileHost = DefaultMobileHost(),
            settingsMirror = SettingsMirror(FakeCore(), FakePreferences()) { true },
        )

    protected class FakePlatformServices : PlatformServices {
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

    protected class FakePreferences : AppPreferences {
        private val strings = mutableMapOf<String, String>()
        private val booleans = mutableMapOf<String, Boolean>()
        private val ints = mutableMapOf<String, Int>()
        private val stringSets = mutableMapOf<String, Set<String>>()

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
        ): Boolean = booleans[key] ?: default

        override fun putBoolean(
            key: String,
            value: Boolean,
        ) {
            booleans[key] = value
        }

        override fun getInt(
            key: String,
            default: Int,
        ): Int = ints[key] ?: default

        override fun putInt(
            key: String,
            value: Int,
        ) {
            ints[key] = value
        }

        override fun getStringSet(
            key: String,
            default: Set<String>,
        ): Set<String> = stringSets[key] ?: default

        override fun putStringSet(
            key: String,
            value: Set<String>,
        ) {
            stringSets[key] = value
        }

        override fun remove(key: String) {
            strings.remove(key)
            booleans.remove(key)
            ints.remove(key)
            stringSets.remove(key)
        }
    }

    protected class FakeLocaleController : LocaleController {
        override fun systemLanguageTag(): String = ""

        override fun applySystem(tag: String) {}

        override fun deviceLanguageTag(): String = "en-US"

        override fun displayName(tag: String): String = tag
    }

    protected class FakeCore : MeronCore {
        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String = "{}"

        override fun events(): CoreEventStream =
            object : CoreEventStream {
                override fun subscribe(listener: (CoreEvent) -> Unit): CloseableHandle = CloseableHandle {}
            }

        override suspend fun protocolVersion(): Int = 0
    }
}
