// At startup the mobile app restores the selected
// account and folder. The cases cover saved mailbox
// navigation and the unified-inbox default on a fresh
// profile.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class MailLocationPrefsTest {
    @Test
    fun defaultsToUnifiedInboxWhenNoMailboxWasSaved() {
        val prefs = MemoryAppPreferences()

        assertEquals(UNIFIED_ACCOUNT_ID, loadLastMailAccountId(prefs))
        assertEquals(INBOX_FOLDER, loadLastMailFolder(prefs))
    }

    @Test
    fun savesAndRestoresAccountMailbox() {
        val prefs = MemoryAppPreferences()

        saveLastMailLocation(prefs, "acct-1", "Archive")

        assertEquals("acct-1", loadLastMailAccountId(prefs))
        assertEquals("Archive", loadLastMailFolder(prefs))
    }

    @Test
    fun aFolderRenameFollowsTheRememberedFolderOfAnotherAccount() {
        val prefs = MemoryAppPreferences()
        saveLastMailLocation(prefs, "acct-1", "Work/Reports")
        saveLastMailLocation(prefs, "acct-2", "Archive")

        renameMailFolderForAccount(prefs, "acct-1", mapOf("Work" to "Jobs", "Work/Reports" to "Jobs/Reports"))
        renameMailFolderForAccount(prefs, "acct-2", mapOf("Work" to "Jobs"))

        assertEquals("Jobs/Reports", loadMailFolderForAccount(prefs, "acct-1"))
        assertEquals("Archive", loadMailFolderForAccount(prefs, "acct-2"))
        // The last-opened account stays as it was.
        assertEquals("acct-2", loadLastMailAccountId(prefs))
    }

    @Test
    fun blankSavedValuesFallBackToUnifiedInbox() {
        val prefs = MemoryAppPreferences()

        saveLastMailLocation(prefs, "", "")

        assertEquals(UNIFIED_ACCOUNT_ID, loadLastMailAccountId(prefs))
        assertEquals(INBOX_FOLDER, loadLastMailFolder(prefs))
    }

    @Test
    fun remembersFoldersIndependentlyForEachAccountAndUnifiedMailbox() {
        val prefs = MemoryAppPreferences()

        saveLastMailLocation(prefs, "acct-1", "Projects/Work")
        saveLastMailLocation(prefs, "acct-2", "Archive")
        saveLastMailLocation(prefs, UNIFIED_ACCOUNT_ID, "starred")

        assertEquals("Projects/Work", loadMailFolderForAccount(prefs, "acct-1"))
        assertEquals("Archive", loadMailFolderForAccount(prefs, "acct-2"))
        assertEquals("starred", loadMailFolderForAccount(prefs, UNIFIED_ACCOUNT_ID))
        assertEquals(INBOX_FOLDER, loadMailFolderForAccount(prefs, "new-account"))
        assertEquals("starred", loadLastMailFolder(prefs))

        saveLastMailLocation(prefs, "acct-1", "Sent")
        assertEquals("Sent", loadMailFolderForAccount(prefs, "acct-1"))
        assertEquals("Archive", loadMailFolderForAccount(prefs, "acct-2"))
        assertEquals("Sent", loadLastMailFolder(prefs))
    }

    @Test
    fun upgradePreservesLegacyFolderWhenSwitchingToAnotherAccount() {
        val prefs = MemoryAppPreferences()
        prefs.putString(LAST_MAIL_ACCOUNT_PREF, "acct-1")
        prefs.putString(LAST_MAIL_FOLDER_PREF, "Projects/Work")

        assertEquals("Projects/Work", loadLastMailFolder(prefs))
        assertEquals(INBOX_FOLDER, loadMailFolderForAccount(prefs, "acct-2"))
        saveLastMailLocation(prefs, "acct-2", "Archive")

        assertEquals("Projects/Work", loadMailFolderForAccount(prefs, "acct-1"))
        assertEquals("Archive", loadLastMailFolder(prefs))
    }

    @Test
    fun migrationOnlyWritesOnceAndNormalSavesUseTwoWrites() {
        val prefs = MemoryAppPreferences()
        prefs.putString(LAST_MAIL_ACCOUNT_PREF, "acct-1")
        prefs.putString(LAST_MAIL_FOLDER_PREF, "Projects/Work")
        val beforeMigration = prefs.stringWrites

        assertEquals("Projects/Work", loadMailFolderForAccount(prefs, "acct-1"))
        assertEquals(beforeMigration, prefs.stringWrites)
        migrateLegacyMailFolder(prefs)
        assertEquals(beforeMigration + 1, prefs.stringWrites)
        assertEquals("", prefs.getString(LAST_MAIL_FOLDER_PREF, ""))
        migrateLegacyMailFolder(prefs)
        loadMailFolderForAccount(prefs, "acct-1")
        assertEquals(beforeMigration + 1, prefs.stringWrites)

        saveLastMailLocation(prefs, "acct-2", "Archive")
        assertEquals(beforeMigration + 3, prefs.stringWrites)
    }

    @Test
    fun clearingAnAccountFolderDoesNotAffectOtherAccounts() {
        val prefs = MemoryAppPreferences()
        saveLastMailLocation(prefs, "acct-1", "Projects/Work")
        saveLastMailLocation(prefs, "acct-2", "Archive")

        clearMailFolderForAccount(prefs, "acct-1")

        assertEquals(INBOX_FOLDER, loadMailFolderForAccount(prefs, "acct-1"))
        assertEquals("Archive", loadMailFolderForAccount(prefs, "acct-2"))
    }

    @Test
    fun clearingLegacyAccountFolderDoesNotRestoreItOnNextLoad() {
        val prefs = MemoryAppPreferences()
        prefs.putString(LAST_MAIL_ACCOUNT_PREF, "acct-1")
        prefs.putString(LAST_MAIL_FOLDER_PREF, "Projects/Work")

        clearMailFolderForAccount(prefs, "acct-1")

        assertEquals(INBOX_FOLDER, loadMailFolderForAccount(prefs, "acct-1"))
    }
}
