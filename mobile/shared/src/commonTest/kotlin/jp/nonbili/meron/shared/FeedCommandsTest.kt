// RSS subscription commands sent by the mobile client.
// Fixtures capture add, move, remove and OPML requests.
// These tests cover both the core command names and their serialized payloads.
package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class FeedCommandsTest {
    @Test
    fun accountAddRssUsesDesktopBridgePayloadShape() {
        val request =
            accountAddRssRequest(
                id = 9,
                params =
                    AddRssAccountParams(
                        feedUrl = "https://example.com/feed.xml",
                        displayName = "News",
                    ),
            )

        assertEquals(
            """{"id":9,"method":"account.addRss","params":{"feed_url":"https://example.com/feed.xml","display_name":"News"}}""",
            request.toJson(),
        )
    }

    @Test
    fun rssClientMethodsUseSharedCoreCommandNames() {
        val core = RecordingCommandCore("""{"ok":true}""")
        val client = MobileMailCommandClient(core)

        runCommandFixture {
            client.addRssAccount(
                AddRssAccountParams(
                    feedUrl = "https://example.com/feed.xml",
                    displayName = "News",
                ),
            )
        }
        assertEquals(MobileCommand.AccountAddRss, core.lastCommand)
        assertEquals(
            """{"feed_url":"https://example.com/feed.xml","display_name":"News"}""",
            core.lastPayloadJson,
        )

        runCommandFixture {
            client.addRssFeed(
                AddRssFeedParams(
                    accountId = "rss-account",
                    feedUrl = "https://example.com/other.xml",
                ),
            )
        }
        assertEquals(MobileCommand.FeedAdd, core.lastCommand)
        assertEquals(
            """{"account":"rss-account","feed_url":"https://example.com/other.xml"}""",
            core.lastPayloadJson,
        )

        runCommandFixture {
            client.removeRssFeed(RemoveRssFeedParams(threadId = "rss-account#rss#feed-1"))
        }
        assertEquals(MobileCommand.FeedRemove, core.lastCommand)
        assertEquals("""{"thread_id":"rss-account#rss#feed-1"}""", core.lastPayloadJson)

        runCommandFixture {
            client.moveRssFeed(
                MoveRssFeedParams(
                    threadId = "rss-account#rss#feed-1",
                    targetAccountId = "other-rss-account",
                ),
            )
        }
        assertEquals(MobileCommand.FeedMove, core.lastCommand)
        assertEquals(
            """{"thread_id":"rss-account#rss#feed-1","target_account":"other-rss-account"}""",
            core.lastPayloadJson,
        )

        runCommandFixture {
            client.exportOpml(ExportOpmlParams(accountId = "rss-account"))
        }
        assertEquals(MobileCommand.FeedExportOpml, core.lastCommand)
        assertEquals("""{"account":"rss-account"}""", core.lastPayloadJson)

        runCommandFixture {
            client.importOpml(ImportOpmlParams(accountId = "rss-account", opml = "<opml version=\"2.0\"/>"))
        }
        assertEquals(MobileCommand.FeedImportOpml, core.lastCommand)
        assertEquals("""{"account":"rss-account","opml":"<opml version=\"2.0\"/>"}""", core.lastPayloadJson)

        runCommandFixture {
            client.syncRss(SyncRssParams(accountId = "rss-account"))
        }
        assertEquals(MobileCommand.RssSync, core.lastCommand)
        assertEquals("""{"account_id":"rss-account"}""", core.lastPayloadJson)

        runCommandFixture {
            client.readRssThread(RssThreadParams(threadId = "rss-account#rss#feed-1"))
        }
        assertEquals(MobileCommand.RssThread, core.lastCommand)
        assertEquals("""{"thread_id":"rss-account#rss#feed-1","limit":10}""", core.lastPayloadJson)

        runCommandFixture {
            client.markRssRead(
                RssMarkReadParams(
                    threadId = "rss-account#rss#feed-1",
                    seen = false,
                    itemKeys = listOf("item-1"),
                ),
            )
        }
        assertEquals(MobileCommand.RssMarkRead, core.lastCommand)
        assertEquals(
            """{"thread_id":"rss-account#rss#feed-1","seen":false,"item_keys":["item-1"]}""",
            core.lastPayloadJson,
        )

        runCommandFixture {
            client.markRssStarred(
                RssMarkStarredParams(
                    threadId = "rss-account#rss#feed-1",
                    starred = true,
                    itemKeys = listOf("item-1"),
                ),
            )
        }
        assertEquals(MobileCommand.RssMarkStarred, core.lastCommand)
        assertEquals(
            """{"thread_id":"rss-account#rss#feed-1","starred":true,"item_keys":["item-1"]}""",
            core.lastPayloadJson,
        )
    }

    @Test
    fun rssCommandsMatchMobileCorePayloadShapes() {
        assertEquals(
            """{"id":31,"method":"feed.remove","params":{"thread_id":"rss-account#rss#feed-1"}}""",
            feedRemoveRequest(
                id = 31,
                params = RemoveRssFeedParams(threadId = "rss-account#rss#feed-1"),
            ).toJson(),
        )
        assertEquals(
            """{"id":32,"method":"feed.move","params":{"thread_id":"rss-account#rss#feed-1","target_account":"target-rss"}}""",
            feedMoveRequest(
                id = 32,
                params =
                    MoveRssFeedParams(
                        threadId = "rss-account#rss#feed-1",
                        targetAccountId = "target-rss",
                    ),
            ).toJson(),
        )
        assertEquals(
            """{"id":33,"method":"rss.sync","params":{"account_id":"rss-account"}}""",
            syncRssRequest(id = 33, params = SyncRssParams(accountId = "rss-account")).toJson(),
        )
        assertEquals(
            """{"id":34,"method":"rss.thread","params":{"thread_id":"rss-account#rss#feed-1","before_cursor":"1700000000:abc","limit":25}}""",
            rssThreadRequest(
                id = 34,
                params =
                    RssThreadParams(
                        threadId = "rss-account#rss#feed-1",
                        beforeCursor = "1700000000:abc",
                        limit = 25,
                    ),
            ).toJson(),
        )
        assertEquals(
            """{"id":35,"method":"rss.markRead","params":{"thread_id":"rss-account#rss#feed-1","seen":false,"item_keys":["item-1","item-2"]}}""",
            rssMarkReadRequest(
                id = 35,
                params =
                    RssMarkReadParams(
                        threadId = "rss-account#rss#feed-1",
                        seen = false,
                        itemKeys = listOf("item-1", "item-2"),
                    ),
            ).toJson(),
        )
        assertEquals(
            """{"id":36,"method":"rss.markStarred","params":{"thread_id":"rss-account#rss#feed-1","starred":true,"item_keys":["item-1"]}}""",
            rssMarkStarredRequest(
                id = 36,
                params =
                    RssMarkStarredParams(
                        threadId = "rss-account#rss#feed-1",
                        starred = true,
                        itemKeys = listOf("item-1"),
                    ),
            ).toJson(),
        )
        assertEquals(
            """{"id":37,"method":"rss.exportOpml","params":{"account":"rss-account"}}""",
            feedExportOpmlRequest(id = 37, params = ExportOpmlParams(accountId = "rss-account")).toJson(),
        )
        assertEquals(
            """{"id":38,"method":"rss.importOpml","params":{"account":"rss-account","opml":"<opml/>"}}""",
            feedImportOpmlRequest(id = 38, params = ImportOpmlParams(accountId = "rss-account", opml = "<opml/>")).toJson(),
        )
        assertEquals(
            """{"id":39,"method":"mail.attachmentRead","params":{"key":"acc/INBOX/1/note.txt"}}""",
            attachmentReadRequest(id = 39, params = AttachmentReadParams(key = "acc/INBOX/1/note.txt")).toJson(),
        )
        assertEquals(
            """{"id":40,"method":"storage.usage","params":{}}""",
            storageUsageRequest(id = 40).toJson(),
        )
        assertEquals(
            """{"id":41,"method":"storage.clearCache","params":{}}""",
            storageClearCacheRequest(id = 41).toJson(),
        )
    }
}
