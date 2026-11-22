// Maps device locales and persisted choices to the catalogs shipped with Meron.
// The shared UI and Android background services both resolve languages here.
// An empty result lets the caller follow the device or use English as appropriate.
package jp.nonbili.meron.shared

fun catalogLanguage(tag: String): String =
    when (val language = tag.substringBefore('-').substringBefore('_').lowercase()) {
        "pt" -> "pt-BR"
        "ru", "en" -> language
        else -> ""
    }
