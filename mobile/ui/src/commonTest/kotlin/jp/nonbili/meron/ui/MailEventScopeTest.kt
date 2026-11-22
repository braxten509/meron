// Background sync events can belong to a different
// account from the open mailbox. These tests check
// which changes schedule a reload of the visible
// folder.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.FolderSummary
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class MailEventScopeTest {
    @Test
    fun inboxEventForTheOpenAccountReloads() {
        assertTrue(
            affects(eventAccount = "acc1", eventFolder = "INBOX", selectedAccountId = "acc1", selectedFolder = "inbox"),
        )
    }

    @Test
    fun eventForAnotherAccountIsIgnored() {
        assertFalse(
            affects(eventAccount = "acc2", eventFolder = "INBOX", selectedAccountId = "acc1", selectedFolder = "inbox"),
        )
    }

    // The foreground watchers IDLE on each account's Sent folder as well, so
    // these arrive on every cold start and must not reload an open inbox.
    @Test
    fun sentFolderEventIsIgnoredWhileViewingTheInbox() {
        assertFalse(
            affects(eventAccount = "acc1", eventFolder = "Sent", selectedAccountId = "acc1", selectedFolder = "inbox"),
        )
    }

    @Test
    fun sentFolderEventReloadsWhileViewingSent() {
        assertTrue(
            affects(eventAccount = "acc1", eventFolder = "Sent", selectedAccountId = "acc1", selectedFolder = "sent"),
        )
    }

    @Test
    fun unifiedReloadsForAnyIncludedAccountInbox() {
        assertTrue(
            affects(
                eventAccount = "acc2",
                eventFolder = "inbox",
                selectedAccountId = UNIFIED_ACCOUNT_ID,
                selectedFolder = INBOX_FOLDER,
                unifiedAccountIds = setOf("acc1", "acc2"),
            ),
        )
    }

    @Test
    fun unifiedIgnoresAccountsExcludedFromIt() {
        assertFalse(
            affects(
                eventAccount = "acc3",
                eventFolder = "inbox",
                selectedAccountId = UNIFIED_ACCOUNT_ID,
                selectedFolder = INBOX_FOLDER,
                unifiedAccountIds = setOf("acc1", "acc2"),
            ),
        )
    }

    @Test
    fun unifiedMatchesTheSelectedRoleThroughTheAccountsRealFolderName() {
        assertTrue(
            affects(
                eventAccount = "acc1",
                eventFolder = "Postausgang",
                selectedAccountId = UNIFIED_ACCOUNT_ID,
                selectedFolder = "sent",
                unifiedAccountIds = setOf("acc1"),
                unifiedFoldersByAccount =
                    mapOf(
                        "acc1" to
                            listOf(
                                FolderSummary(
                                    accountId = "acc1",
                                    name = "Postausgang",
                                    role = "sent",
                                ),
                            ),
                    ),
            ),
        )
    }

    // A starred item can sit in any folder, so the starred listing reloads on
    // any event from an account it covers — including one from a folder the
    // unified view has no role for.
    @Test
    fun unifiedStarredReloadsForAnyFolderOfAnIncludedAccount() {
        assertTrue(
            affects(
                eventAccount = "acc1",
                eventFolder = "Some/Custom/Folder",
                selectedAccountId = UNIFIED_ACCOUNT_ID,
                selectedFolder = STARRED_FOLDER,
                unifiedAccountIds = setOf("acc1"),
            ),
        )
        assertFalse(
            affects(
                eventAccount = "acc3",
                eventFolder = "INBOX",
                selectedAccountId = UNIFIED_ACCOUNT_ID,
                selectedFolder = STARRED_FOLDER,
                unifiedAccountIds = setOf("acc1"),
            ),
        )
    }

    // Folder-list syncs emit {account, folders:true} with no folder at all.
    @Test
    fun eventWithoutAFolderReloads() {
        assertTrue(
            affects(eventAccount = "acc1", eventFolder = "", selectedAccountId = "acc1", selectedFolder = "sent"),
        )
    }

    @Test
    fun eventWithoutAnAccountReloads() {
        assertTrue(
            affects(eventAccount = "", eventFolder = "", selectedAccountId = "acc1", selectedFolder = "inbox"),
        )
    }

    @Test
    fun blankSelectionIsTreatedAsUnified() {
        assertTrue(
            affects(
                eventAccount = "acc1",
                eventFolder = "inbox",
                selectedAccountId = "",
                selectedFolder = "",
                unifiedAccountIds = setOf("acc1"),
            ),
        )
    }

    private fun affects(
        eventAccount: String,
        eventFolder: String,
        selectedAccountId: String,
        selectedFolder: String,
        unifiedAccountIds: Set<String> = emptySet(),
        unifiedFoldersByAccount: Map<String, List<FolderSummary>> = emptyMap(),
    ): Boolean =
        mailEventAffectsVisibleMailbox(
            eventAccount = eventAccount,
            eventFolder = eventFolder,
            selectedAccountId = selectedAccountId,
            selectedFolder = selectedFolder,
            unifiedAccountIds = unifiedAccountIds,
            unifiedFoldersByAccount = unifiedFoldersByAccount,
        )
}
