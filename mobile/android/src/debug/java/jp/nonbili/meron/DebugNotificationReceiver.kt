// Receives debug-only broadcast actions used to exercise
// Android notifications. The debug application can inspect
// notification behavior separately from a normal mailbox
// refresh.

package jp.nonbili.meron

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import org.json.JSONArray
import org.json.JSONObject

/**
 * Posts sample new-mail notifications in debug builds.
 *
 * Debug-only: posts a fake `mail.newMessages` batch so the shade layout, the group summary and the tap
 * targets can be checked without waiting for real mail to arrive. Debug source set only — never in a
 * release build.
 *
 * Send a canned batch:
 *     adb shell am broadcast -a jp.nonbili.meron.DEBUG_NEW_MAIL \
 *         -n jp.nonbili.meron/.DebugNotificationReceiver --ei count 3
 *
 * Or a payload of your own, shaped like a `mail.newMessages` detail:
 *     adb shell am broadcast -a jp.nonbili.meron.DEBUG_NEW_MAIL \
 *         -n jp.nonbili.meron/.DebugNotificationReceiver \
 *         --es detail '{"account":"a1","accountName":"me@example.com",...}'
 *
 * `account`/`folder` should match a real account for the tap-through to land on a mailbox; anything else
 * still renders.
 *
 * Raise the mailbox-changed event on its own, to check that a running app reloads when something changes
 * behind its back:
 *     adb shell am broadcast -a jp.nonbili.meron.DEBUG_MAILBOX_CHANGED \
 *         -n jp.nonbili.meron/.DebugNotificationReceiver \
 *         --es account <account-id> --es folder INBOX
 */
class DebugNotificationReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        if (intent.action == "jp.nonbili.meron.DEBUG_MAILBOX_CHANGED") {
            MeronCoreNative.dispatchLocalEvent(
                JSONObject()
                    .put("event", MAILBOX_CHANGED_EVENT)
                    .put(
                        "detail",
                        JSONObject()
                            .put("account", intent.getStringExtra("account").orEmpty())
                            .put("folder", intent.getStringExtra("folder").orEmpty().ifBlank { "INBOX" }),
                    ).toString(),
            )
            return
        }
        val detail =
            intent.getStringExtra("detail")?.let { JSONObject(it) }
                ?: sampleDetail(
                    account = intent.getStringExtra("account").orEmpty().ifBlank { "debug-account" },
                    accountName = intent.getStringExtra("accountName").orEmpty().ifBlank { "debug@example.com" },
                    folder = intent.getStringExtra("folder").orEmpty().ifBlank { "INBOX" },
                    count = intent.getIntExtra("count", 3),
                )
        AndroidNotificationService.notifyNewMail(context, detail)
    }

    private fun sampleDetail(
        account: String,
        accountName: String,
        folder: String,
        count: Int,
    ): JSONObject {
        // Fresh uids per broadcast, so repeated sends stack instead of
        // replacing the notifications already in the shade.
        val base = System.currentTimeMillis() / 1000
        // Start at a different sample each broadcast: the group summary lists
        // distinct "sender - subject" rows, so repeating a batch verbatim would
        // collapse into the rows already showing.
        val rotation = (base % SAMPLE_SENDERS.size).toInt()
        val messages = JSONArray()
        repeat(count.coerceAtLeast(1)) { offset ->
            val index = rotation + offset
            val sender = SAMPLE_SENDERS[index % SAMPLE_SENDERS.size]
            val subject = SAMPLE_SUBJECTS[index % SAMPLE_SUBJECTS.size]
            messages.put(
                JSONObject()
                    .put("uid", base + offset)
                    .put("from", sender)
                    .put("subject", subject)
                    .put("preview", SAMPLE_PREVIEWS[index % SAMPLE_PREVIEWS.size])
                    .put("threadKey", "debug-thread-${base + offset}")
                    .put("date", base - offset * 60),
            )
        }
        return JSONObject()
            .put("account", account)
            .put("accountName", accountName)
            .put("folder", folder)
            .put("count", messages.length())
            .put("messages", messages)
    }

    private companion object {
        val SAMPLE_SENDERS =
            listOf(
                "Aiko Tanaka",
                "build-bot@ci.example.com",
                "Marco Silva",
                "Newsletter Weekly",
            )
        val SAMPLE_SUBJECTS =
            listOf(
                "Lunch on Thursday?",
                "Nightly build #4821 failed",
                "Re: contract draft",
                "This week in mail clients",
            )
        val SAMPLE_PREVIEWS =
            listOf(
                "There's a new place near the station that does a proper set menu, want to try it?",
                "The arm64 job timed out after 40 minutes. Logs are attached to the run.",
                "Thanks for the redline — I only had one comment on clause 4, otherwise it looks good to sign.",
                "Five things worth reading, and one long piece on why IMAP IDLE is the way it is.",
            )
    }
}
