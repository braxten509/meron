// Estimates the cost of sorting a thousand messages.
// The settings screen applies provider prices to header and message token estimates.
// Formatting keeps small estimates readable in the model picker.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import jp.nonbili.meron.shared.AiModel
import kotlin.math.pow
import kotlin.math.roundToLong

// Rough size of one new email's request, as in the plan's cost table: about
// 300 tokens in for sender and subject, 125 more with the start of the email,
// and 30 out.
private const val TOKENS_IN_HEADERS = 300.0

private const val TOKENS_IN_START = 425.0

private const val TOKENS_OUT = 30.0

// A decision model reads one email and the folder list per request, and bills
// no output.
private const val DECISION_TOKENS_IN_HEADERS = 150.0

private const val DECISION_TOKENS_IN_START = 275.0

/** About what 1,000 new emails cost with [model], or null when its price is unknown. */
internal fun costPerThousand(
    model: AiModel,
    reads: String,
): Double? {
    val priceIn = model.inputPricePerMillion ?: return null
    if (model.decision) {
        val tokensIn = if (reads == READS_START) DECISION_TOKENS_IN_START else DECISION_TOKENS_IN_HEADERS
        return 1000 * tokensIn * priceIn / 1_000_000
    }
    val priceOut = model.outputPricePerMillion ?: return null
    val tokensIn = if (reads == READS_START) TOKENS_IN_START else TOKENS_IN_HEADERS
    return 1000 * (tokensIn * priceIn + TOKENS_OUT * priceOut) / 1_000_000
}

/** US dollars with two significant digits below a dollar: $0.042, $0.50, $1.20. */
internal fun formatUsd(value: Double): String {
    if (value <= 0.0) return "\$0"
    var decimals = 2
    if (value < 1) {
        var scaled = value
        decimals = 0
        while (scaled < 10 && decimals < 8) {
            scaled *= 10
            decimals += 1
        }
        decimals = maxOf(decimals, 2)
    }
    val factor = 10.0.pow(decimals).toLong()
    val units = (value * factor).roundToLong()
    val fraction = (units % factor).toString().padStart(decimals, '0')
    val trimmed = if (decimals > 2) fraction.trimEnd('0').padEnd(2, '0') else fraction
    return "\$${units / factor}.$trimmed"
}
