// Preference Parameters for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

/**
 * Settings to read from the core store. The `settings` table is authoritative
 * for mobile preferences; the platform store in front of it is only a cache so
 * the first frame can paint before the keyed DB is open.
 */
data class AppPrefsGetParams(
    val keys: List<String>,
) {
    fun toJson(): String = jsonObject("keys" to keys.jsonStringArray())
}

/**
 * One setting to persist. [valueJson] is already-encoded JSON so a preference
 * can be any of the shapes the store holds (string, bool, number, array).
 */
data class AppPrefsSetParams(
    val key: String,
    val valueJson: String,
) {
    fun toJson(): String =
        jsonObject(
            "key" to key.jsonString(),
            "value" to valueJson.ifBlank { "null" },
        )
}
