// Resolves the signature used by a mobile sending
// account. Draft creation considers the global setting,
// account override and whether stored markup has visible
// content.

package jp.nonbili.meron.shared

// Signatures are stored as HTML — one app-wide signature in the core `settings`
// table, plus an optional per-account override — and are inserted into the draft
// body when a composer opens, so what the user sees is what goes out.
//
// The mobile composer edits plain text, so a signature reaches it through
// [signaturePlainText]. Rich markup written on desktop survives untouched as
// long as the text is not edited here (see the settings screen).
//
// The thread's quick reply is seeded the same way, matching desktop. It renders
// as a chat bubble, but what it sends is an ordinary mail that its recipient
// reads in an ordinary client, so leaving it unsigned reads as inconsistency
// rather than brevity. Its tracking is simpler — see quickReplySignature.

/** Where a signature lands relative to whatever the draft was seeded with. */
enum class SignaturePlacement {
    /** Above a seeded quote, so a forward keeps the quote last. */
    AboveQuote,

    /** Below text the user already typed (a quick reply carried into compose). */
    BelowText,
}

/**
 * The signature an account actually sends: its own override, nothing when it
 * opts out, or the app-wide signature. Blank signatures resolve to "".
 */
fun resolveSignatureHtml(
    account: AccountSummary?,
    appSignatureHtml: String,
): String {
    val signature = account?.signature ?: SignatureSpec.followApp
    val html =
        when (signature.mode) {
            "none" -> ""
            "custom" -> signature.html
            else -> appSignatureHtml
        }
    return if (signatureIsBlank(html)) "" else html
}

/** Whether signature HTML carries any visible content (text or an image). */
fun signatureIsBlank(html: String): Boolean {
    if (html.isBlank()) return true
    val stripped =
        html
            .replace(Regex("<(img|hr|br)\\b[^>]*>", RegexOption.IGNORE_CASE), "x")
            .replace(Regex("<[^>]*>"), "")
            .replace("&nbsp;", " ", ignoreCase = true)
    return stripped.isBlank()
}

private val BLOCK_TAG = Regex("</(p|div|li|tr|h[1-6]|blockquote|pre)>", RegexOption.IGNORE_CASE)
private val LINE_BREAK = Regex("<(br|hr)\\s*/?>", RegexOption.IGNORE_CASE)

/**
 * A signature's plaintext form: block elements end a line, `<br>` breaks one,
 * and the remaining markup and entities are unwrapped. Deliberately simple —
 * signatures are short, and anything richer belongs in the HTML alternative
 * the desktop composer sends.
 */
fun signaturePlainText(html: String): String {
    if (html.isBlank()) return ""
    return html
        .replace(BLOCK_TAG, "\n")
        .replace(LINE_BREAK, "\n")
        .replace(Regex("<[^>]*>"), "")
        .replace("&nbsp;", " ", ignoreCase = true)
        .replace("&lt;", "<", ignoreCase = true)
        .replace("&gt;", ">", ignoreCase = true)
        .replace("&quot;", "\"", ignoreCase = true)
        .replace("&#39;", "'")
        .replace("&amp;", "&", ignoreCase = true)
        .lines()
        .joinToString("\n") { it.trim() }
        .trim('\n')
        .trim()
}

/** A signature this app put in a draft body, and where it put it. */
data class SignatureMark(
    val text: String,
    val placement: SignaturePlacement,
)

/**
 * What a draft knows about the signature in its body — three states, not two:
 *
 *   a mark with text   the app inserted exactly this, there; it can be swapped
 *   a mark without     the app inserted nothing, because the account sends no
 *                      signature. The placement is still remembered, so a later
 *                      account with one puts it where this draft's would have
 *                      gone — above the quote of a forward, not after it.
 *   null               unmanaged. The body came from elsewhere (a saved draft
 *                      reopened, "edit as new") and may well already end in a
 *                      signature. Nothing here is ours to rewrite, and
 *                      appending would give the message two.
 */
typealias SignatureTracking = SignatureMark?

/** The mark for a draft the app deliberately gave no signature. */
fun noSignatureMark(placement: SignaturePlacement = SignaturePlacement.BelowText) = SignatureMark("", placement)

/** The mark for a fresh draft with no signature and no reason to place one. */
val NO_SIGNATURE = noSignatureMark()

/** The body and tracking a draft ends up with after a change of account. */
data class SwappedSignature(
    val body: String,
    val tracking: SignatureTracking,
)

/**
 * Move a draft body to another account's signature, reporting what the draft
 * now knows about it. A signature that is no longer in the body verbatim has
 * been edited — the text is the user's now, so it is neither replaced nor
 * tracked any further.
 */
fun bodyWithSwappedSignature(
    body: String,
    tracking: SignatureTracking,
    next: String,
): SwappedSignature {
    // Unmanaged: leave the body alone, and keep it unmanaged.
    if (tracking == null) return SwappedSignature(body, null)

    fun markFor(placement: SignaturePlacement) = if (next.isBlank()) noSignatureMark(placement) else SignatureMark(next, placement)

    // Nothing inserted yet: this is the first signature the draft gets, and it
    // goes where this draft's signature belongs — which a forward opened under
    // an account with no signature of its own still remembers.
    if (tracking.text.isEmpty()) {
        return SwappedSignature(bodyWithSignature(body, next, tracking.placement), markFor(tracking.placement))
    }

    val range = signatureRange(body, tracking) ?: return SwappedSignature(body, null)
    val prefix = body.substring(0, range.first)
    val suffix = body.substring(range.last + 1)
    val swapped = if (next.isBlank()) joinAcrossRemoval(prefix, suffix) else prefix + next + suffix
    return SwappedSignature(swapped, markFor(tracking.placement))
}

/**
 * Where the tracked signature sits in [body] as whole lines of its own, or null
 * when it cannot be identified — because the user has edited it, or because the
 * body holds a second block just like it and ours is no longer where it was put.
 *
 * The line boundaries tell an untouched signature apart from one the user has
 * written into: "Ping" must not match inside "Ping, but edited". The placement
 * says which copy is ours when there are several — above a quote the first,
 * below the text the last — but that alone is a guess, and guessing wrong would
 * rewrite the user's own words. So in an ambiguous body ours must still be
 * exactly at the edge it was inserted against to be touched at all; failing
 * that the draft simply stops being managed.
 */
private fun signatureRange(
    body: String,
    mark: SignatureMark,
): IntRange? {
    val matches = blockMatches(body, mark.text)
    if (matches.isEmpty()) return null

    val index = if (mark.placement == SignaturePlacement.AboveQuote) matches.first() else matches.last()
    val range = index until index + mark.text.length
    if (matches.size == 1) return range

    val edge =
        if (mark.placement == SignaturePlacement.AboveQuote) {
            body.substring(0, index)
        } else {
            body.substring(range.last + 1)
        }
    return if (edge.isBlank()) range else null
}

/** Every position where [needle] sits in [body] as whole lines of its own. */
private fun blockMatches(
    body: String,
    needle: String,
): List<Int> {
    if (needle.isEmpty()) return emptyList()
    val out = mutableListOf<Int>()
    var index = body.indexOf(needle)
    while (index >= 0) {
        val end = index + needle.length
        val startsLine = index == 0 || body[index - 1] == '\n'
        val endsLine = end == body.length || body[end] == '\n'
        if (startsLine && endsLine) out.add(index)
        index = body.indexOf(needle, index + 1)
    }
    return out
}

/**
 * Rejoin a body after cutting the signature out of it, closing the blank line
 * that separated it. Only the seam is touched: blank lines elsewhere are the
 * user's text (or a quote's), and rewriting those is not our business.
 */
private fun joinAcrossRemoval(
    prefix: String,
    suffix: String,
): String {
    val head = prefix.trimEnd('\n')
    val tail = suffix.trimStart('\n')
    // A signature above a quote sat under a blank line meant for typing; that
    // line is not part of the signature, so it stays.
    if (head.isEmpty()) return if (tail.isEmpty()) "" else "\n\n$tail"
    return if (tail.isEmpty()) head else "$head\n\n$tail"
}

/**
 * Place a signature in a plaintext draft body, leaving a blank line above it for
 * the cursor. Returns [body] unchanged when there is no signature.
 */
fun bodyWithSignature(
    body: String,
    signatureText: String,
    placement: SignaturePlacement = SignaturePlacement.BelowText,
): String {
    if (signatureText.isBlank()) return body
    if (body.isBlank()) return "\n\n$signatureText"
    // Only the blank line at the join is this function's business: whitespace
    // the user (or a quote) put elsewhere in the body stays exactly as it is.
    return when (placement) {
        SignaturePlacement.AboveQuote -> "\n\n$signatureText\n\n${body.trimStart('\n')}"
        SignaturePlacement.BelowText -> "${body.trimEnd('\n')}\n\n$signatureText"
    }
}
