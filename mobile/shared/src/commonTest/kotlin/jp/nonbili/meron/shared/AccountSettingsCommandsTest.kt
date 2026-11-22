// Settings updates sent from mobile to the shared core.
// The command fixtures capture method names and parameters as the client sends them.
// The same settings must be readable by desktop and mobile hosts.
package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class AccountSettingsCommandsTest {
    @Test
    fun accountSettingCommandsUseDesktopBridgePayloadShapes() {
        assertEquals(
            """{"id":11,"method":"account.remove","params":{"id":"acc1"}}""",
            accountRemoveRequest(id = 11, params = AccountIdParams(accountId = "acc1")).toJson(),
        )
        assertEquals(
            """{"id":12,"method":"account.setName","params":{"id":"acc1","name":"Personal"}}""",
            accountSetNameRequest(id = 12, params = AccountNameParams(accountId = "acc1", name = "Personal")).toJson(),
        )
        assertEquals(
            """{"id":13,"method":"account.setSenderName","params":{"id":"acc1","name":"Sender"}}""",
            accountSetSenderNameRequest(id = 13, params = AccountNameParams(accountId = "acc1", name = "Sender")).toJson(),
        )
        assertEquals(
            """{"id":14,"method":"account.setAvatar","params":{"id":"acc1","avatar_url":"https://example.com/avatar.png"}}""",
            accountSetAvatarRequest(
                id = 14,
                params = AccountAvatarParams(accountId = "acc1", avatarUrl = "https://example.com/avatar.png"),
            ).toJson(),
        )
        assertEquals(
            """{"id":15,"method":"account.writeAvatarFile","params":{"id":"acc1","filename":"avatar.png","mime":"image/png","data":"aGVsbG8="}}""",
            accountWriteAvatarFileRequest(
                id = 15,
                params = AccountMediaFileParams(accountId = "acc1", filename = "avatar.png", mime = "image/png", data = "aGVsbG8="),
            ).toJson(),
        )
        assertEquals(
            """{"id":16,"method":"account.setChatWallpaper","params":{"id":"acc1","wallpaper":{"kind":"preset","presetId":"grid"}}}""",
            accountSetChatWallpaperRequest(id = 16, params = AccountChatWallpaperParams(accountId = "acc1", presetId = "grid")).toJson(),
        )
        assertEquals(
            """{"id":17,"method":"account.writeChatWallpaperFile","params":{"id":"acc1","filename":"wallpaper.png","mime":"image/png","data":"aGVsbG8="}}""",
            accountWriteChatWallpaperFileRequest(
                id = 17,
                params = AccountMediaFileParams(accountId = "acc1", filename = "wallpaper.png", mime = "image/png", data = "aGVsbG8="),
            ).toJson(),
        )
        assertEquals(
            """{"id":18,"method":"account.setChatWallpaper","params":{"id":"acc1","wallpaper":null}}""",
            accountSetChatWallpaperRequest(id = 18, params = AccountChatWallpaperParams(accountId = "acc1")).toJson(),
        )
        assertEquals(
            """{"id":19,"method":"account.setImages","params":{"id":"acc1","enabled":true}}""",
            accountSetImagesRequest(id = 19, params = AccountFlagParams(accountId = "acc1", enabled = true)).toJson(),
        )
        assertEquals(
            """{"id":20,"method":"account.setConversationHtml","params":{"id":"acc1","enabled":false}}""",
            accountSetConversationHtmlRequest(id = 20, params = AccountFlagParams(accountId = "acc1", enabled = false)).toJson(),
        )
        assertEquals(
            """{"id":21,"method":"account.setUnified","params":{"id":"acc1","enabled":false}}""",
            accountSetUnifiedRequest(id = 21, params = AccountFlagParams(accountId = "acc1", enabled = false)).toJson(),
        )
        assertEquals(
            """{"id":22,"method":"account.setMuted","params":{"id":"acc1","enabled":true}}""",
            accountSetMutedRequest(id = 22, params = AccountFlagParams(accountId = "acc1", enabled = true)).toJson(),
        )
        assertEquals(
            """{"id":23,"method":"account.setPaused","params":{"id":"acc1","enabled":true}}""",
            accountSetPausedRequest(id = 23, params = AccountFlagParams(accountId = "acc1", enabled = true)).toJson(),
        )
        assertEquals(
            """{"id":24,"method":"account.setRSSSyncInterval","params":{"id":"rss1","minutes":30}}""",
            accountSetRssSyncIntervalRequest(id = 24, params = AccountRssSyncIntervalParams(accountId = "rss1", minutes = 30)).toJson(),
        )
        assertEquals(
            """{"id":25,"method":"account.setAliases","params":{"id":"acc1","aliases":[{"email":"alias@example.com","name":"Alias"}]}}""",
            accountSetAliasesRequest(
                id = 25,
                params =
                    AccountAliasesParams(
                        accountId = "acc1",
                        aliases = listOf(AccountAliasParams(email = "alias@example.com", name = "Alias")),
                    ),
            ).toJson(),
        )
        assertEquals(
            """{"id":60,"method":"account.setProxy","params":{"id":"acc1","proxy":{"mode":"socks5","host":"127.0.0.1","port":9050,"username":"u","password":"p"}}}""",
            setAccountProxyRequest(
                id = 60,
                params =
                    AccountProxyParams(
                        accountId = "acc1",
                        proxy = ProxySpec(mode = "socks5", host = "127.0.0.1", port = 9050, username = "u", password = "p"),
                    ),
            ).toJson(),
        )
        assertEquals(
            """{"id":61,"method":"app.proxyGet","params":{}}""",
            proxyGetRequest(id = 61).toJson(),
        )
        assertEquals(
            """{"id":62,"method":"app.proxySet","params":{"proxy":{"mode":"off","host":"","port":0,"username":"","password":""}}}""",
            proxySetRequest(id = 62, params = ProxyParams(ProxySpec.off)).toJson(),
        )
        assertEquals(
            """{"id":26,"method":"account.reorder","params":{"accounts":["acc2","acc1"]}}""",
            accountReorderRequest(id = 26, params = AccountReorderParams(listOf("acc2", "acc1"))).toJson(),
        )
        assertEquals(
            """{"id":27,"method":"mail.suggestContacts","params":{"account":"acc1","query":"bea","limit":8}}""",
            contactSuggestRequest(id = 27, params = ContactSuggestParams(accountId = "acc1", query = "bea")).toJson(),
        )
    }

    @Test
    fun accountSettingClientMethodsUseSharedCoreCommandNames() {
        val core = RecordingCommandCore("""{"ok":true}""")
        val client = MobileMailCommandClient(core)

        runCommandFixture { client.setAccountName(AccountNameParams(accountId = "acc1", name = "Personal")) }
        assertEquals(MobileCommand.AccountSetName, core.lastCommand)
        assertEquals("""{"id":"acc1","name":"Personal"}""", core.lastPayloadJson)

        runCommandFixture { client.setAccountAvatar(AccountAvatarParams(accountId = "acc1", avatarUrl = "https://example.com/avatar.png")) }
        assertEquals(MobileCommand.AccountSetAvatar, core.lastCommand)
        assertEquals("""{"id":"acc1","avatar_url":"https://example.com/avatar.png"}""", core.lastPayloadJson)

        runCommandFixture {
            client.writeAccountAvatarFile(
                AccountMediaFileParams(accountId = "acc1", filename = "avatar.png", mime = "image/png", data = "aGVsbG8="),
            )
        }
        assertEquals(MobileCommand.AccountWriteAvatarFile, core.lastCommand)
        assertEquals("""{"id":"acc1","filename":"avatar.png","mime":"image/png","data":"aGVsbG8="}""", core.lastPayloadJson)

        runCommandFixture { client.setAccountChatWallpaper(AccountChatWallpaperParams(accountId = "acc1", presetId = "grid")) }
        assertEquals(MobileCommand.AccountSetChatWallpaper, core.lastCommand)
        assertEquals("""{"id":"acc1","wallpaper":{"kind":"preset","presetId":"grid"}}""", core.lastPayloadJson)

        runCommandFixture {
            client.writeAccountChatWallpaperFile(
                AccountMediaFileParams(accountId = "acc1", filename = "wallpaper.png", mime = "image/png", data = "aGVsbG8="),
            )
        }
        assertEquals(MobileCommand.AccountWriteChatWallpaperFile, core.lastCommand)
        assertEquals("""{"id":"acc1","filename":"wallpaper.png","mime":"image/png","data":"aGVsbG8="}""", core.lastPayloadJson)

        runCommandFixture { client.setAccountUnified(AccountFlagParams(accountId = "acc1", enabled = false)) }
        assertEquals(MobileCommand.AccountSetUnified, core.lastCommand)
        assertEquals("""{"id":"acc1","enabled":false}""", core.lastPayloadJson)

        runCommandFixture { client.setAccountAliases(AccountAliasesParams("acc1", listOf(AccountAliasParams("alias@example.com", "Alias")))) }
        assertEquals(MobileCommand.AccountSetAliases, core.lastCommand)
        assertEquals("""{"id":"acc1","aliases":[{"email":"alias@example.com","name":"Alias"}]}""", core.lastPayloadJson)

        runCommandFixture { client.setAccountProxy(AccountProxyParams("acc1", ProxySpec.followApp)) }
        assertEquals(MobileCommand.AccountSetProxy, core.lastCommand)
        assertEquals("""{"id":"acc1","proxy":{"mode":"global","host":"","port":0,"username":"","password":""}}""", core.lastPayloadJson)

        runCommandFixture { client.getProxy() }
        assertEquals(MobileCommand.AppProxyGet, core.lastCommand)

        runCommandFixture { client.setProxy(ProxyParams(ProxySpec(mode = "http", host = "gateway.corp", port = 3128))) }
        assertEquals(MobileCommand.AppProxySet, core.lastCommand)
        assertEquals(
            """{"proxy":{"mode":"http","host":"gateway.corp","port":3128,"username":"","password":""}}""",
            core.lastPayloadJson,
        )

        runCommandFixture { client.reorderAccounts(AccountReorderParams(listOf("acc2", "acc1"))) }
        assertEquals(MobileCommand.AccountReorder, core.lastCommand)
        assertEquals("""{"accounts":["acc2","acc1"]}""", core.lastPayloadJson)

        runCommandFixture { client.suggestContacts(ContactSuggestParams(accountId = "acc1", query = "bea", limit = 4)) }
        assertEquals(MobileCommand.ContactSuggest, core.lastCommand)
        assertEquals("""{"account":"acc1","query":"bea","limit":4}""", core.lastPayloadJson)
    }
}
