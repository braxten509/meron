// Reads fields and slices from the JSON returned by mobile core commands.
// Callers can distinguish exact top-level fields from a search through a wrapped response.
// The reader tracks string escapes and balanced containers while preserving raw nested values.
package jp.nonbili.meron.shared

internal fun String.fields(): Map<String, String> = if (trim().startsWith("{")) jsonObjectEntries().toMap() else emptyMap()

/** A command's own fields. The core hands every answer back wrapped as
 *  `{"id":…,"result":{…}}`, so read inside `result` when it is there (and the
 *  object itself when a caller already unwrapped it). */
internal fun String.responseFields(): Map<String, String> {
    val top = fields()
    return top["result"]?.takeIf { it.startsWith('{') }?.fields() ?: top
}

internal fun Map<String, String>.stringField(name: String): String? = this[name]?.takeIf { it.startsWith('"') }?.readResponseString(0)?.value

internal fun Map<String, String>.bool(name: String): Boolean? =
    when (this[name]) {
        "true" -> true
        "false" -> false
        else -> null
    }

internal fun Map<String, String>.number(name: String): Double? = this[name]?.let { raw -> raw.toDoubleOrNull() ?: stringField(name)?.toDoubleOrNull() }

internal fun Map<String, String>.int(name: String): Int? = number(name)?.toInt()

internal fun Map<String, String>.objectField(name: String): Map<String, String>? = this[name]?.takeIf { it.startsWith('{') }?.fields()

internal fun Map<String, String>.list(name: String): List<String> = this[name]?.takeIf { it.startsWith('[') }?.jsonArrayElements().orEmpty()

internal fun String.topLevelStringArray(name: String): List<String> =
    jsonObjectEntries()
        .firstOrNull { it.first == name }
        ?.second
        ?.takeIf { it.startsWith("[") }
        ?.jsonArrayElements()
        ?.mapNotNull { element -> "{\"value\":$element}".findJsonStringProperty("value") }
        .orEmpty()

internal fun String.findJsonArrayProperty(name: String): String? {
    val key = name.jsonString()
    var index = 0
    while (index < length) {
        val found = indexOf(key, startIndex = index)
        if (found < 0) return null
        var cursor = skipResponseWhitespace(found + key.length)
        if (cursor < length && this[cursor] == ':') {
            cursor = skipResponseWhitespace(cursor + 1)
            if (cursor < length && this[cursor] == '[') {
                return readBalancedJson(cursor, '[', ']').value
            }
        }
        index = found + key.length
    }
    return null
}

internal fun String.findJsonObjectProperty(name: String): String? {
    val key = name.jsonString()
    var index = 0
    while (index < length) {
        val found = indexOf(key, startIndex = index)
        if (found < 0) return null
        var cursor = skipResponseWhitespace(found + key.length)
        if (cursor < length && this[cursor] == ':') {
            cursor = skipResponseWhitespace(cursor + 1)
            if (cursor < length && this[cursor] == '{') {
                return readBalancedJson(cursor, '{', '}').value
            }
        }
        index = found + key.length
    }
    return null
}

/** Top-level `"key": value` pairs of a JSON object, values left unparsed. */
internal fun String.jsonObjectEntries(): List<Pair<String, String>> {
    val body = trim().removePrefix("{").removeSuffix("}")
    val out = mutableListOf<Pair<String, String>>()
    var index = 0
    while (index < body.length) {
        index = body.skipResponseSeparators(index)
        if (index >= body.length || body[index] != '"') break
        val key = body.readResponseString(index)
        index = body.skipResponseWhitespace(key.nextIndex)
        if (index >= body.length || body[index] != ':') break
        index = body.skipResponseWhitespace(index + 1)
        if (index >= body.length) break
        val value = body.readResponseValue(index)
        out += key.value to value.value
        index = value.nextIndex
    }
    return out
}

// ---- Calendar ---------------------------------------------------------------

internal fun String.findJsonStringProperty(
    name: String,
    topLevelOnly: Boolean = false,
): String? {
    val value = (if (topLevelOnly) findTopLevelJsonPropertyValue(name) else findJsonPropertyValue(name)) ?: return null
    if (!value.startsWith('"')) return null
    return value.readResponseString(0).value
}

internal fun String.findJsonBooleanProperty(
    name: String,
    topLevelOnly: Boolean = false,
): Boolean? =
    when (if (topLevelOnly) findTopLevelJsonPropertyValue(name) else findJsonPropertyValue(name)) {
        "true" -> true
        "false" -> false
        else -> null
    }

internal fun String.findJsonLongProperty(name: String): Long? = findJsonPropertyValue(name)?.toLongOrNull()

// Mutation responses also contain change.thread_id, which identifies the
// source. The action's location fields belong to the outer object only.
private fun String.findTopLevelJsonPropertyValue(name: String): String? {
    var cursor = skipResponseWhitespace(0)
    if (getOrNull(cursor) != '{') return null
    cursor = skipResponseWhitespace(cursor + 1)
    while (cursor < length && this[cursor] != '}') {
        val key = readResponseString(cursor)
        cursor = skipResponseWhitespace(key.nextIndex)
        require(getOrNull(cursor) == ':') { "Expected JSON property value" }
        val value = readResponseValue(skipResponseWhitespace(cursor + 1))
        if (key.value == name) return value.value
        cursor = skipResponseWhitespace(value.nextIndex)
        if (getOrNull(cursor) != ',') return null
        cursor = skipResponseWhitespace(cursor + 1)
    }
    return null
}

internal fun String.findJsonPropertyValue(name: String): String? {
    val key = name.jsonString()
    var index = 0
    while (index < length) {
        val found = indexOf(key, startIndex = index)
        if (found < 0) return null
        var cursor = skipResponseWhitespace(found + key.length)
        if (cursor < length && this[cursor] == ':') {
            cursor = skipResponseWhitespace(cursor + 1)
            return readResponseValue(cursor).value
        }
        index = found + key.length
    }
    return null
}

internal data class ResponseJsonSlice(
    val value: String,
    val nextIndex: Int,
)

internal fun String.jsonArrayElements(): List<String> {
    val body = trim().removePrefix("[").removeSuffix("]")
    val out = mutableListOf<String>()
    var index = 0
    while (index < body.length) {
        index = body.skipResponseSeparators(index)
        if (index >= body.length) break
        val value = body.readResponseValue(index)
        out += value.value
        index = value.nextIndex
    }
    return out
}

private fun String.skipResponseSeparators(start: Int): Int {
    var index = start
    while (index < length && (this[index].isWhitespace() || this[index] == ',')) index += 1
    return index
}

private fun String.skipResponseWhitespace(start: Int): Int {
    var index = start
    while (index < length && this[index].isWhitespace()) index += 1
    return index
}

internal fun String.readResponseString(start: Int): ResponseJsonSlice {
    require(start < length && this[start] == '"') { "Expected JSON string" }
    val out = StringBuilder()
    var index = start + 1
    while (index < length) {
        when (val ch = this[index]) {
            '"' -> {
                return ResponseJsonSlice(out.toString(), index + 1)
            }

            '\\' -> {
                require(index + 1 < length) { "Dangling JSON escape" }
                when (val escaped = this[index + 1]) {
                    '"', '\\', '/' -> {
                        out.append(escaped)
                    }

                    'b' -> {
                        out.append('\b')
                    }

                    'f' -> {
                        out.append('\u000c')
                    }

                    'n' -> {
                        out.append('\n')
                    }

                    'r' -> {
                        out.append('\r')
                    }

                    't' -> {
                        out.append('\t')
                    }

                    'u' -> {
                        require(index + 5 < length) { "Short unicode escape" }
                        out.append(substring(index + 2, index + 6).toInt(16).toChar())
                        index += 4
                    }

                    else -> {
                        error("Bad JSON escape: \\$escaped")
                    }
                }
                index += 2
            }

            else -> {
                out.append(ch)
                index += 1
            }
        }
    }
    error("Unterminated JSON string")
}

private fun String.readResponseValue(start: Int): ResponseJsonSlice {
    require(start < length) { "Expected JSON value" }
    return when (this[start]) {
        '"' -> {
            val string = readResponseString(start)
            ResponseJsonSlice(substring(start, string.nextIndex), string.nextIndex)
        }

        '{' -> {
            readBalancedJson(start, '{', '}')
        }

        '[' -> {
            readBalancedJson(start, '[', ']')
        }

        else -> {
            var index = start
            while (index < length && this[index] != ',' && this[index] != '}' && this[index] != ']') index += 1
            ResponseJsonSlice(substring(start, index).trim(), index)
        }
    }
}

private fun String.readBalancedJson(
    start: Int,
    open: Char,
    close: Char,
): ResponseJsonSlice {
    var depth = 0
    var index = start
    var inString = false
    while (index < length) {
        val ch = this[index]
        if (inString) {
            when (ch) {
                '"' -> inString = false
                '\\' -> index += 1
            }
        } else {
            when (ch) {
                '"' -> {
                    inString = true
                }

                open -> {
                    depth += 1
                }

                close -> {
                    depth -= 1
                    if (depth == 0) return ResponseJsonSlice(substring(start, index + 1), index + 1)
                }
            }
        }
        index += 1
    }
    error("Unbalanced JSON value")
}
