// Parameter Encoding for the mobile protocol.
// Used by the shared Kotlin client when it calls into the Rust core.
// Wire keys and request identifiers must stay compatible across platforms.
package jp.nonbili.meron.shared

internal fun jsonObject(vararg fields: Pair<String, String?>): String =
    fields
        .filter { it.second != null }
        .joinToString(separator = ",", prefix = "{", postfix = "}") { (name, value) -> "${name.jsonString()}:$value" }

internal fun List<String>.jsonStringArray(): String = joinToString(separator = ",", prefix = "[", postfix = "]") { it.jsonString() }
