// Mobile backup commands send host identity, passphrase
// and export options to the shared core. The fixtures
// also round-trip settings through the backup
// representation.

package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class BackupCommandsTest {
    @Test
    fun exportBackupParamsSerializeSecretsFlagPassphraseAndHostIdentity() {
        assertEquals(
            """{"include_secrets":false,"passphrase":"","app_version":"","platform":""}""",
            ExportBackupParams().toJson(),
        )
        assertEquals(
            """{"include_secrets":true,"passphrase":"correct horse","app_version":"0.2.4","platform":"android"}""",
            ExportBackupParams(
                includeSecrets = true,
                passphrase = "correct horse",
                appVersion = "0.2.4",
                platform = "android",
            ).toJson(),
        )
    }

    @Test
    fun settingValuesRoundTripThroughJson() {
        val values =
            mapOf<String, Any>(
                "mobile.app.appearance_mode_v1" to "Indigo",
                "mobile.app.message_font_scale_v1" to 115,
                "mobile.app.show_unread_badges_v1" to true,
                "mobile.app.background_sync_enabled_v1" to false,
                "mobile.app.hidden_navigation_accounts_v1" to listOf("a@example.com", "b@example.com"),
                "mobile.kanban.kanban_boards_v1" to """[{"id":"b1","name":"Work"}]""",
            )

        // The store writes one key at a time, so rebuild the object the way a
        // prefsGet response carries it.
        val encoded =
            values.entries.joinToString(",", "{", "}") { (key, value) ->
                "\"$key\":${encodeAppPrefValue(value)}"
            }
        val decoded = parseAppPrefsResponse("""{"prefs":$encoded}""")

        assertEquals("Indigo", decoded["mobile.app.appearance_mode_v1"])
        // JSON numbers decode as Long.
        assertEquals(115L, decoded["mobile.app.message_font_scale_v1"])
        assertEquals(true, decoded["mobile.app.show_unread_badges_v1"])
        assertEquals(false, decoded["mobile.app.background_sync_enabled_v1"])
        assertEquals(
            listOf("a@example.com", "b@example.com"),
            decoded["mobile.app.hidden_navigation_accounts_v1"],
        )
        // A string that is itself JSON must survive escaping intact.
        assertEquals("""[{"id":"b1","name":"Work"}]""", decoded["mobile.kanban.kanban_boards_v1"])
    }

    @Test
    fun settingValuesEncodeToTheirJsonTypes() {
        assertEquals(""""Indigo"""", encodeAppPrefValue("Indigo"))
        assertEquals("true", encodeAppPrefValue(true))
        assertEquals("115", encodeAppPrefValue(115))
        assertEquals("""["a","b"]""", encodeAppPrefValue(listOf("a", "b")))
        assertEquals("[]", encodeAppPrefValue(emptyList<String>()))
    }

    @Test
    fun parsingPrefsToleratesAMissingOrOddPayload() {
        assertEquals(emptyMap(), parseAppPrefsResponse("""{"accounts":1}"""))
        assertEquals(emptyMap(), parseAppPrefsResponse("not json"))
        // Values the pref store cannot hold are skipped, not guessed at.
        assertEquals(
            emptyMap(),
            parseAppPrefsResponse("""{"prefs":{"a":null,"b":{"nested":1},"c":1.5}}"""),
        )
    }

    @Test
    fun importResultCarriesTheCounts() {
        val result =
            parseBackupImportResponse(
                """{"accounts":1,"skipped":0,"feeds":0,"settings":3,"secrets":1}""",
            )
        assertEquals(1, result.accounts)
        assertEquals(3, result.settings)
        assertEquals(1, result.secrets)
    }

    @Test
    fun importBackupParamsEscapeTheEmbeddedDocument() {
        // The backup is JSON inside a JSON string, so its quotes must escape.
        assertEquals(
            """{"backup":"{\"meron_backup\":1}","passphrase":"pw"}""",
            ImportBackupParams(backup = """{"meron_backup":1}""", passphrase = "pw").toJson(),
        )
    }

    @Test
    fun backupRequestsUseTheProtocolMethodNames() {
        assertEquals(
            """{"id":90,"method":"backup.export","params":{"include_secrets":true,"passphrase":"pw","app_version":"1.2.3","platform":"ios"}}""",
            backupExportRequest(
                id = 90,
                params =
                    ExportBackupParams(
                        includeSecrets = true,
                        passphrase = "pw",
                        appVersion = "1.2.3",
                        platform = "ios",
                    ),
            ).toJson(),
        )
        assertEquals(
            """{"id":91,"method":"backup.import","params":{"backup":"{}","passphrase":""}}""",
            backupImportRequest(id = 91, params = ImportBackupParams(backup = "{}")).toJson(),
        )
    }

    @Test
    fun parseBackupExportResponseReadsTheDocument() {
        assertEquals(
            """{"meron_backup":1}""",
            parseBackupExportResponse("""{"backup":"{\"meron_backup\":1}"}"""),
        )
        assertEquals("", parseBackupExportResponse("""{"other":"value"}"""))
        assertEquals("", parseBackupExportResponse("not json"))
    }

    @Test
    fun parseBackupImportResponseReadsTheCounts() {
        val result =
            parseBackupImportResponse(
                """{"accounts":2,"skipped":1,"feeds":5,"settings":9,"secrets":2}""",
            )
        assertFalse(result.needsPassphrase)
        assertEquals(2, result.accounts)
        assertEquals(1, result.skipped)
        assertEquals(5, result.feeds)
        assertEquals(9, result.settings)
        assertEquals(2, result.secrets)
    }

    @Test
    fun parseBackupImportResponseFlagsAnEncryptedFile() {
        val result = parseBackupImportResponse("""{"needs_passphrase":true}""")
        assertTrue(result.needsPassphrase)
        // Nothing was restored, so the counts stay at zero.
        assertEquals(0, result.accounts)
    }

    @Test
    fun parseBackupImportResponseDefaultsMissingCountsToZero() {
        val result = parseBackupImportResponse("{}")
        assertFalse(result.needsPassphrase)
        assertEquals(0, result.accounts)
        assertEquals(0, result.skipped)
        assertEquals(0, result.feeds)
        assertEquals(0, result.settings)
        assertEquals(0, result.secrets)
    }
}
