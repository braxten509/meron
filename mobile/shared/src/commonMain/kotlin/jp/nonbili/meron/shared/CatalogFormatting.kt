// Formats the ICU messages used by the shared UI and mobile host services.
// Plurals can occur inside a sentence, followed by a destination or other arguments.
// The catalog language selects the plural form before named fields are substituted.
package jp.nonbili.meron.shared

fun formatCatalogMessage(
    template: String,
    args: Map<String, Any?>,
    locale: String,
): String {
    // A plural can sit anywhere in the text ("Moved {count, plural, …} to
    // {name}"), and there can be more than one: resolve each in turn.
    var text = template
    repeat(MAX_PLURALS_PER_STRING) {
        val plural = parsePlural(text) ?: return replacePlaceholders(text, args)
        val count =
            (args[plural.argument] as? Number)?.toLong()
                ?: args[plural.argument]?.toString()?.toLongOrNull()
                ?: 0L
        val category =
            when (locale.substringBefore('-').substringBefore('_').lowercase()) {
                "ru" -> russianPluralCategory(count)
                else -> if (count == 1L) "one" else "other"
            }
        val chosen = plural.variants[category] ?: plural.variants["other"] ?: return replacePlaceholders(text, args)
        text = text.substring(0, plural.start) + chosen + text.substring(plural.end)
    }
    return replacePlaceholders(text, args)
}

private const val MAX_PLURALS_PER_STRING = 4

private fun russianPluralCategory(count: Long): String {
    val lastTwo = kotlin.math.abs(count % 100)
    val last = lastTwo % 10
    return when {
        last == 1L && lastTwo != 11L -> "one"
        last in 2L..4L && lastTwo !in 12L..14L -> "few"
        else -> "many"
    }
}

private data class IcuPlural(
    val argument: String,
    val variants: Map<String, String>,
    /** Where the `{arg, plural, …}` block starts and ends (exclusive) in the text. */
    val start: Int,
    val end: Int,
)

private fun parsePlural(template: String): IcuPlural? {
    val prefix = Regex("""\{([A-Za-z][A-Za-z0-9_]*)\s*,\s*plural\s*,""").find(template) ?: return null
    var index = prefix.range.last + 1
    val variants = linkedMapOf<String, String>()
    while (index < template.length) {
        while (index < template.length && template[index].isWhitespace()) index += 1
        val categoryStart = index
        while (index < template.length && template[index].isLetter()) index += 1
        if (categoryStart == index) break
        val category = template.substring(categoryStart, index)
        while (index < template.length && template[index].isWhitespace()) index += 1
        if (index >= template.length || template[index] != '{') break
        val branch = readBraceBranch(template, index)
        variants[category] = branch.first
        index = branch.second
    }
    while (index < template.length && template[index].isWhitespace()) index += 1
    // The block's own closing brace; without one the text is malformed, so
    // treat everything to the end as the block, as before.
    val end = if (index < template.length && template[index] == '}') index + 1 else template.length
    return IcuPlural(prefix.groupValues[1], variants, prefix.range.first, end)
}

private fun readBraceBranch(
    template: String,
    start: Int,
): Pair<String, Int> {
    var depth = 0
    var index = start
    while (index < template.length) {
        when (template[index]) {
            '{' -> {
                depth += 1
            }

            '}' -> {
                depth -= 1
                if (depth == 0) {
                    return template.substring(start + 1, index) to index + 1
                }
            }
        }
        index += 1
    }
    return template.substring(start + 1) to template.length
}

private fun replacePlaceholders(
    value: String,
    args: Map<String, Any?>,
): String =
    Regex("""\{([A-Za-z][A-Za-z0-9_]*)(?:\s*,\s*number)?\}""").replace(value) { match ->
        args[match.groupValues[1]]?.toString().orEmpty()
    }
