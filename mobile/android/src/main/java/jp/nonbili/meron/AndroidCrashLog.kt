// Records crash details for the Android diagnostic
// view. Exception types and available messages are
// condensed into entries that can be read after
// restart.

package jp.nonbili.meron

import android.content.Context
import java.io.File
import java.io.PrintWriter
import java.io.StringWriter
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** Cap the captured trace so one crash cannot flush the whole rolling log. */
private const val MAX_TRACE_LINES = 60

/** One-line "type: message" identification of a crash, for the report prompt. */
internal fun crashSummary(error: Throwable): String = "${error::class.java.name}: ${error.message.orEmpty()}".trim().trimEnd(':')

/** Keep the outer exception plus the deepest cause and its first frames. */
internal fun truncateTrace(trace: String): String {
    val lines = trace.trimEnd().lines()
    if (lines.size <= MAX_TRACE_LINES) return lines.joinToString("\n")

    val headLines = MAX_TRACE_LINES / 2
    val deepestCause = lines.indexOfLast { it.trimStart().startsWith("Caused by:") }
    val tail =
        if (deepestCause >= headLines) {
            lines.drop(deepestCause).take(MAX_TRACE_LINES - headLines)
        } else {
            lines.takeLast(MAX_TRACE_LINES - headLines)
        }
    val kept = lines.take(headLines) + tail
    return (lines.take(headLines) + "... ${lines.size - kept.size} frames omitted" + tail).joinToString("\n")
}

/**
 * Captures fatal crashes into the shareable diagnostic log, and leaves a marker
 * so the next launch can offer to send the report.
 *
 * Deliberately local-only: nothing leaves the device unless the user picks
 * "Send report" and reviews the share sheet, which is why the app carries no
 * crash-reporting SDK.
 */
object AndroidCrashLog {
    private const val MARKER_FILE_NAME = "pending-crash.txt"

    private val timestampFormat = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.US)

    /**
     * Install the process-wide uncaught exception handler. Call once, from
     * [MeronApplication], so background sync and the push service are covered
     * too — not just the activity.
     */
    fun install(context: Context) {
        val appContext = context.applicationContext
        installCorePanicListener(appContext)
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            runCatching { record(appContext, "thread '${thread.name}'", error) }
            // Chain so the platform still shows the crash dialog and reports to
            // Play Vitals; swallowing it here would hide the crash from both.
            previous?.uncaughtException(thread, error)
        }
    }

    /**
     * Keep the native event callback registered even when no UI exists. Without
     * this listener a Rust panic during WorkManager sync has nowhere to send its
     * final log event before the process aborts.
     */
    private fun installCorePanicListener(context: Context) {
        MeronCoreNative.addCoreEventListener { eventJson ->
            runCatching {
                val event = org.json.JSONObject(eventJson)
                if (event.optString("event") != "log") return@runCatching
                val detail = event.optJSONObject("detail") ?: return@runCatching
                if (detail.optString("tag") != "panic") return@runCatching
                val message = detail.optString("message", "core panic")
                AndroidSyncDiagnosticLog.appendRedacted(context, "E core/panic $message")
                markPending(context, message)
            }
        }
    }

    /** Write [error] to the diagnostic log and mark the crash as unreported. */
    fun record(
        context: Context,
        where: String,
        error: Throwable,
    ) {
        val summary = crashSummary(error)
        AndroidSyncDiagnosticLog.appendRedacted(context, "FATAL $where: $summary\n${traceOf(error)}")
        markPending(context, summary)
    }

    /**
     * Note a crash whose stack trace is already in the diagnostic log — a Rust
     * core panic aborts the process without ever reaching the JVM handler, so
     * the panic log line is all we get.
     */
    @Synchronized
    fun markPending(
        context: Context,
        summary: String,
    ) {
        val timestamp = timestampFormat.format(Date())
        runCatching {
            markerFile(context).writeText("$timestamp ${redactMessage(summary)}\n")
        }
    }

    /**
     * One-line summary of the last unreported crash, or "" when the previous
     * run ended normally (or the user already dismissed the prompt).
     */
    fun pending(context: Context): String {
        val file = markerFile(context)
        return runCatching { if (file.exists()) file.readText().trim() else "" }.getOrDefault("")
    }

    /** Drop the marker, so the prompt is shown once per crash. */
    fun clearPending(context: Context) {
        runCatching { markerFile(context).delete() }
    }

    private fun traceOf(error: Throwable): String {
        val writer = StringWriter()
        error.printStackTrace(PrintWriter(writer))
        return truncateTrace(writer.toString())
    }

    private fun markerFile(context: Context): File = File(context.filesDir, MARKER_FILE_NAME)
}
