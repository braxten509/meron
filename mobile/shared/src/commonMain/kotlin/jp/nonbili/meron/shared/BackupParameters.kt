// Backup Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

/**
 * Backup export options.
 *
 * A passphrase encrypts the document; [includeSecrets] additionally puts account passwords and OAuth
 * tokens in it, which the core refuses to do without a passphrase.
 *
 * [appVersion] is the host bundle's version name and [platform] is "android" or "ios"; both are
 * recorded in the backup envelope. The core knows neither: its crate version is not the app's, and
 * desktop and mobile are versioned on separate tracks, so the number only identifies a build alongside
 * the platform that wrote it.
 */
data class ExportBackupParams(
    val includeSecrets: Boolean = false,
    val passphrase: String = "",
    val appVersion: String = "",
    val platform: String = "",
) {
    fun toJson(): String =
        jsonObject(
            "include_secrets" to includeSecrets.toString(),
            "passphrase" to passphrase.jsonString(),
            "app_version" to appVersion.jsonString(),
            "platform" to platform.jsonString(),
        )
}

data class ImportBackupParams(
    val backup: String,
    val passphrase: String = "",
) {
    fun toJson(): String =
        jsonObject(
            "backup" to backup.jsonString(),
            "passphrase" to passphrase.jsonString(),
        )
}
