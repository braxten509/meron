package jp.nonbili.meron.ui

import platform.Foundation.NSLocale
import platform.Foundation.NSUserDefaults
import platform.Foundation.localizedStringForLocaleIdentifier
import platform.Foundation.preferredLanguages

/** iOS in-app locale via the AppleLanguages default. */
class IosLocaleController : LocaleController {
    private val defaults = NSUserDefaults.standardUserDefaults

    // iOS has no per-app language the user can set outside the app, so the stored
    // tag is always the one that decides.
    override fun systemLanguageTag(): String? = null

    override fun applySystem(tag: String) {
        val normalized = normalizeAppLanguageTag(tag)
        if (normalized.isBlank()) {
            defaults.removeObjectForKey("AppleLanguages")
        } else {
            defaults.setObject(listOf(normalized), forKey = "AppleLanguages")
        }
    }

    // preferredLanguages is the user's ordered language list; its head is what the
    // system would pick for an app with no preference of its own.
    override fun deviceLanguageTag(): String = (NSLocale.preferredLanguages.firstOrNull() as? String).orEmpty()

    override fun displayName(tag: String): String {
        val locale = NSLocale(localeIdentifier = tag)
        return locale.localizedStringForLocaleIdentifier(tag).replaceFirstChar { it.uppercaseChar() }
    }
}
