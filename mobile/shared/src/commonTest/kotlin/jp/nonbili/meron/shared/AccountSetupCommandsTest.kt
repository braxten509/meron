// Account setup through the mobile command client.
// Fixtures capture server discovery, independent security modes and OAuth exchanges.
// Expected payloads match the fields accepted by the Rust core.
package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class AccountSetupCommandsTest {
    @Test
    fun accountListUsesDesktopBridgeMethodName() {
        assertEquals(
            """{"id":3,"method":"account.list","params":{}}""",
            accountListRequest(id = 3).toJson(),
        )
    }

    @Test
    fun accountListClientUsesSharedCoreCommand() {
        val core = RecordingCommandCore("""{"accounts":[]}""")
        val response =
            runCommandFixture {
                MobileMailCommandClient(core).listAccounts()
            }

        assertEquals(MobileCommand.AccountList, core.lastCommand)
        assertEquals("{}", core.lastPayloadJson)
        assertEquals("""{"accounts":[]}""", response)
    }

    @Test
    fun accountAddPasswordUsesDesktopBridgePayloadShape() {
        val request =
            accountAddPasswordRequest(
                id = 7,
                params =
                    AddPasswordAccountParams(
                        email = "me@example.com",
                        displayName = "Me",
                        senderName = "Sender",
                        imapHost = "imap.example.com",
                        imapPort = 993,
                        smtpHost = "smtp.example.com",
                        smtpPort = 587,
                        username = "me@example.com",
                        password = "secret",
                        tls = true,
                    ),
            )

        assertEquals(
            """{"id":7,"method":"account.addPassword","params":{"email":"me@example.com","display_name":"Me","sender_name":"Sender","imap_host":"imap.example.com","imap_port":993,"smtp_host":"smtp.example.com","smtp_port":587,"username":"me@example.com","password":"secret","tls":true}}""",
            request.toJson(),
        )
    }

    @Test
    fun accountAddPasswordClientUsesSharedCoreCommand() {
        val core = RecordingCommandCore("""{"account":{"id":"me@example.com"}}""")
        val response =
            runCommandFixture {
                MobileMailCommandClient(core).addPasswordAccount(
                    AddPasswordAccountParams(
                        email = "me@example.com",
                        imapHost = "imap.example.com",
                        smtpHost = "smtp.example.com",
                        username = "me@example.com",
                        password = "secret",
                    ),
                )
            }

        assertEquals(MobileCommand.AccountAddPassword, core.lastCommand)
        assertEquals(
            """{"email":"me@example.com","display_name":"","sender_name":"","imap_host":"imap.example.com","imap_port":993,"smtp_host":"smtp.example.com","smtp_port":465,"username":"me@example.com","password":"secret","tls":true}""",
            core.lastPayloadJson,
        )
        assertEquals("""{"account":{"id":"me@example.com"}}""", response)
    }

    @Test
    fun accountAddPasswordSerializesIndependentSecurityModes() {
        val params =
            AddPasswordAccountParams(
                email = "self-hosted@example.com",
                imapHost = "mail.example.com",
                imapPort = 143,
                smtpHost = "mail.example.com",
                smtpPort = 2525,
                username = "self-hosted",
                password = "secret",
                tls = false,
                starttls = true,
                smtpTls = false,
                smtpStarttls = false,
            )

        assertEquals(
            """{"email":"self-hosted@example.com","display_name":"","sender_name":"","imap_host":"mail.example.com","imap_port":143,"smtp_host":"mail.example.com","smtp_port":2525,"username":"self-hosted","password":"secret","tls":false,"starttls":true,"smtp_tls":false,"smtp_starttls":false}""",
            params.toJson(),
        )
    }

    @Test
    fun accountAutodiscoverClientUsesSharedCoreCommand() {
        val core = RecordingCommandCore("""{"imap_host":"imap.example.com"}""")
        val response =
            runCommandFixture {
                MobileMailCommandClient(core).autodiscoverAccount(AutodiscoverAccountParams("me@example.com"))
            }

        assertEquals(MobileCommand.AccountAutodiscover, core.lastCommand)
        assertEquals("""{"email":"me@example.com"}""", core.lastPayloadJson)
        assertEquals("""{"imap_host":"imap.example.com"}""", response)
    }

    @Test
    fun accountAddOAuthUsesDesktopBridgePayloadShape() {
        val request =
            accountAddOAuthRequest(
                id = 8,
                params =
                    AddOAuthAccountParams(
                        email = "me@gmail.com",
                        provider = "gmail",
                        displayName = "Me",
                        senderName = "Sender",
                        username = "me@gmail.com",
                        avatarUrl = "https://example.com/avatar.png",
                        accessToken = "access",
                        refreshToken = "refresh",
                        tokenExpiresAt = 1_700_000_000,
                    ),
            )

        assertEquals(
            """{"id":8,"method":"account.addOAuth","params":{"email":"me@gmail.com","provider":"gmail","display_name":"Me","sender_name":"Sender","username":"me@gmail.com","avatar_url":"https://example.com/avatar.png","access_token":"access","refresh_token":"refresh","token_expires_at":1700000000}}""",
            request.toJson(),
        )
    }

    @Test
    fun accountAddOAuthClientUsesSharedCoreCommand() {
        val core = RecordingCommandCore("""{"account":{"id":"me@gmail.com"}}""")
        val response =
            runCommandFixture {
                MobileMailCommandClient(core).addOAuthAccount(
                    AddOAuthAccountParams(
                        email = "me@gmail.com",
                        provider = "gmail",
                        refreshToken = "refresh",
                    ),
                )
            }

        assertEquals(MobileCommand.AccountAddOAuth, core.lastCommand)
        assertEquals(
            """{"email":"me@gmail.com","provider":"gmail","display_name":"","sender_name":"","refresh_token":"refresh","token_expires_at":0}""",
            core.lastPayloadJson,
        )
        assertEquals("""{"account":{"id":"me@gmail.com"}}""", response)
    }

    @Test
    fun accountExchangeOAuthCodeUsesMobileCorePayloadShape() {
        val request =
            accountExchangeOAuthCodeRequest(
                id = 10,
                params =
                    ExchangeOAuthCodeParams(
                        email = "me@gmail.com",
                        provider = "gmail",
                        displayName = "Me",
                        senderName = "Sender",
                        code = "auth-code",
                        clientId = "client",
                        clientSecret = "secret",
                        redirectUri = defaultOAuthRedirectUri(),
                        codeVerifier = "verifier",
                    ),
            )

        assertEquals(
            """{"id":10,"method":"account.exchangeOAuthCode","params":{"email":"me@gmail.com","provider":"gmail","display_name":"Me","sender_name":"Sender","code":"auth-code","client_id":"client","client_secret":"secret","redirect_uri":"jp.nonbili.meron.oauth://oauth","code_verifier":"verifier"}}""",
            request.toJson(),
        )
    }

    @Test
    fun accountExchangeOAuthCodeClientUsesSharedCoreCommand() {
        val core = RecordingCommandCore("""{"account":{"id":"me@gmail.com"}}""")
        val response =
            runCommandFixture {
                MobileMailCommandClient(core).exchangeOAuthCode(
                    ExchangeOAuthCodeParams(
                        email = "me@gmail.com",
                        provider = "gmail",
                        code = "auth-code",
                        clientId = "client",
                        redirectUri = defaultOAuthRedirectUri(),
                        codeVerifier = "verifier",
                    ),
                )
            }

        assertEquals(MobileCommand.AccountExchangeOAuthCode, core.lastCommand)
        assertEquals(
            """{"email":"me@gmail.com","provider":"gmail","display_name":"","sender_name":"","code":"auth-code","client_id":"client","redirect_uri":"jp.nonbili.meron.oauth://oauth","code_verifier":"verifier"}""",
            core.lastPayloadJson,
        )
        assertEquals("""{"account":{"id":"me@gmail.com"}}""", response)
    }

    @Test
    fun accountAutodiscoverUsesDesktopBridgePayloadShape() {
        assertEquals(
            """{"id":10,"method":"account.autodiscover","params":{"email":"me@example.com"}}""",
            accountAutodiscoverRequest(id = 10, params = AutodiscoverAccountParams("me@example.com")).toJson(),
        )
    }
}
