// Resolves catalog text for Compose screens and state changes outside composition.
// Language selection and plural formatting are shared with the native hosts.
// Composable and state helpers supply the preference or current UI locale.
package jp.nonbili.meron.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import jp.nonbili.meron.shared.catalogLanguage
import jp.nonbili.meron.shared.formatCatalogMessage
import androidx.compose.runtime.staticCompositionLocalOf

/** Current app language tag (for example, "en", "pt-BR" or "ru"). Provided by the root. */
val LocalAppLocale = staticCompositionLocalOf { "en" }

/** Resolve and ICU-format a catalog string for [locale]; falls back to English
 *  then the key itself. */
internal fun localizedString(
    locale: String,
    key: String,
    args: Map<String, Any?> = emptyMap(),
): String {
    val language = catalogLanguage(locale).ifEmpty { "en" }
    val messages = generatedStrings[language] ?: generatedStrings["en"].orEmpty()
    val template = messages[key] ?: generatedStrings["en"]?.get(key) ?: key
    return formatCatalogMessage(template, args, language)
}

/** Composable string lookup keyed by the current [LocalAppLocale]. */
@Composable
@ReadOnlyComposable
internal fun tr(
    key: String,
    args: Map<String, Any?> = emptyMap(),
): String = localizedString(LocalAppLocale.current, key, args)

/**
 * Catalog lookup outside a composition. `tr` needs `LocalAppLocale`, which the
 * state functions have no access to, so resolve the tag from prefs the same way
 * the root composable does.
 */
internal fun MeronMobileState.trs(
    key: String,
    args: Map<String, Any?> = emptyMap(),
): String = localizedString(loadAppLanguageTag(prefs).ifBlank { "en" }, key, args)

/**
 * [trs] for a failure, followed by its reason when [error] has one: "Move
 * failed: connection reset". The joining punctuation is itself a catalog
 * string, since languages differ on it.
 */
internal fun MeronMobileState.trsError(
    key: String,
    error: Throwable?,
    args: Map<String, Any?> = emptyMap(),
): String = withReason(trs(key, args), error?.message) { template, fields -> trs(template, fields) }

/** [message], then [reason] if there is one, joined by `common.errorWithReason`. */
internal fun withReason(
    message: String,
    reason: String?,
    lookup: (String, Map<String, Any?>) -> String,
): String =
    if (reason.isNullOrBlank()) {
        message
    } else {
        lookup("common.errorWithReason", mapOf("message" to message, "error" to reason))
    }

/** Composable lookup for catalog strings that use positional `%1$s` / `%1$d`
 *  placeholders (the simple/non-ICU arg'd messages). */
@Composable
@ReadOnlyComposable
internal fun trf(
    key: String,
    vararg formatArgs: Any?,
): String = formatPositional(localizedString(LocalAppLocale.current, key), formatArgs)

private fun formatPositional(
    template: String,
    args: Array<out Any?>,
): String {
    var out = template
    args.forEachIndexed { index, value ->
        val text = value?.toString().orEmpty()
        out = out.replace("%${index + 1}\$s", text).replace("%${index + 1}\$d", text)
    }
    return out
}
