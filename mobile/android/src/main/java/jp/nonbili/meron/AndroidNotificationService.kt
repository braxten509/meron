// Builds and manages Android notifications for
// mail arrivals. Core event data supplies the
// target, preview text and account for each
// displayed item.

package jp.nonbili.meron

import android.content.Context
import android.os.Build
import jp.nonbili.meron.shared.accountIdIsRss
import org.json.JSONObject
import java.util.Locale

/** One new mail, as the shade shows it. Built from a `mail.newMessages` entry. */
data class NewMailItem(
    val uid: Long,
    val from: String,
    val subject: String,
    val preview: String,
    val threadKey: String,
    val date: Long,
    /** A feed entry's stable key, empty for mail (which has a uid instead).
     *  A feed is one thread, so this is all that tells its entries apart. */
    val itemKey: String = "",
    /** The sender's exact address, lowercase; blank for a feed or an older
     *  core. A tap opens their chat in group-by-sender mode. */
    val fromAddr: String = "",
)

/** A batch of arrivals for one account: one notification per item, under a
 *  per-account group summary. */
data class NewMailBatch(
    val accountId: String,
    val accountName: String,
    val folder: String,
    val count: Int,
    val items: List<NewMailItem>,
)

/** Most lines an expanded group summary lists; beyond this the shade elides
 *  them anyway and `count` in the collapsed line carries the total. */
const val NEW_MAIL_SUMMARY_LINES = 6

/** Read a `mail.newMessages` detail into a batch.
 *
 *  `messages` carries one entry per arrival, mail and feed alike. A core older
 *  than the per-message payload sends only the top-level summary fields, which
 *  degrade to a single notification for the batch. */
fun parseNewMailBatch(detail: JSONObject): NewMailBatch {
    val accountId = detail.optString("account")
    val listed = detail.optJSONArray("messages")
    val items =
        if (listed != null && listed.length() > 0) {
            (0 until listed.length()).mapNotNull { index ->
                listed.optJSONObject(index)?.let { entry ->
                    NewMailItem(
                        uid = entry.optLong("uid"),
                        itemKey = entry.optString("itemKey"),
                        from = entry.optString("from"),
                        subject = entry.optString("subject"),
                        preview = entry.optString("preview"),
                        threadKey = entry.optString("threadKey"),
                        date = entry.optLong("date"),
                        fromAddr = entry.optString("from_addr"),
                    )
                }
            }
        } else {
            listOf(
                NewMailItem(
                    uid = 0,
                    from = detail.optString("from"),
                    subject = detail.optString("subject"),
                    preview = detail.optString("preview"),
                    threadKey = detail.optString("threadKey"),
                    date = 0,
                    fromAddr = detail.optString("from_addr"),
                ),
            )
        }
    return NewMailBatch(
        accountId = accountId,
        accountName = detail.optString("accountName"),
        folder = detail.optString("folder"),
        count = detail.optInt("count", items.size),
        items = items,
    )
}

/** Notifications for one account share a group, so the shade collapses that
 *  account's arrivals under a single summary instead of interleaving accounts. */
fun newMailGroupKey(accountId: String): String = "$NEW_MAIL_GROUP_PREFIX$accountId"

internal const val NEW_MAIL_GROUP_PREFIX = "jp.nonbili.meron.NEW_MAIL:"

/** The core store row a notification stands for, as `mail.stillUnread` takes
 *  it: mail by folder and UID, a feed entry by its subscription (the item's
 *  thread key) and entry key. Null for a payload carrying neither, which
 *  cannot be checked and so is left to the user. */
fun newMailStoreKey(
    batch: NewMailBatch,
    item: NewMailItem,
): Pair<String, String>? =
    when {
        accountIdIsRss(batch.accountId) -> {
            (item.threadKey to item.itemKey).takeIf { item.threadKey.isNotBlank() && item.itemKey.isNotBlank() }
        }

        item.uid > 0 && batch.folder.isNotBlank() -> {
            batch.folder to item.uid.toString()
        }

        else -> {
            null
        }
    }

/**
 * Generate the stable notification ID for a delivered message.
 *
 * Stable per-message id: re-posting the same mail (a retried sync, a push and a periodic refresh racing)
 * updates its notification instead of stacking a duplicate. A feed entry has no UID but carries its own
 * key, scoped by its feed the way core scopes the stored row — an entry key is a hash of the GUID, unique
 * only within one subscription, and two feeds can syndicate the same post.
 *
 * Only a payload with neither falls back to the thread key, which cannot separate two same-titled entries
 * of one feed.
 */
fun newMailNotificationId(
    accountId: String,
    item: NewMailItem,
): Int =
    if (item.uid > 0) {
        "$accountId#uid:${item.uid}".hashCode()
    } else if (item.itemKey.isNotBlank()) {
        "$accountId#item:${item.threadKey}#${item.itemKey}".hashCode()
    } else {
        "$accountId#thread:${item.threadKey}#${item.subject}".hashCode()
    }

fun newMailSummaryId(accountId: String): Int = "$accountId#summary".hashCode()

/** Sender, or the account when the envelope carried no From at all. */
fun newMailChildTitle(
    from: String,
    accountName: String,
    fallback: String = "New mail",
): String = from.trim().ifBlank { accountName.trim().ifBlank { fallback } }

/** Collapsed line: subject and the start of the body, the way the shade shows a
 *  message before it is expanded. */
fun newMailChildText(
    subject: String,
    preview: String,
    fallback: String = "New mail arrived",
): String {
    val parts = listOf(subject, preview).map { it.trim() }.filter { it.isNotEmpty() }
    return parts.joinToString(" - ").ifBlank { fallback }
}

/** Expanded body: the subject on its own line above the body snippet, so the
 *  mail is readable without opening the app. */
fun newMailChildBigText(
    subject: String,
    preview: String,
    fallback: String = "New mail arrived",
): String {
    val parts = listOf(subject, preview).map { it.trim() }.filter { it.isNotEmpty() }
    return parts.joinToString("\n").ifBlank { fallback }
}

/** Whether a new-mail row offers Archive. A feed item has nowhere to be filed —
 *  core keeps no archive folder for an RSS account, and `mail.archive` does not
 *  route feed threads — so feeds get Mark as read alone. Matches the mail list,
 *  where an RSS row cannot be archive-swiped either. */
fun newMailSupportsArchive(accountId: String): Boolean = !accountIdIsRss(accountId)

/** One summary row: who wrote, and about what. */
fun newMailInboxLine(
    from: String,
    subject: String,
): String {
    val parts = listOf(from, subject).map { it.trim() }.filter { it.isNotEmpty() }
    return parts.joinToString(" - ")
}

internal fun Context.resourceLanguageTag(): String {
    val configuration = resources.configuration
    val locale =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            configuration.locales.takeIf { !it.isEmpty }?.get(0)
        } else {
            @Suppress("DEPRECATION")
            configuration.locale
        }
    return catalogLanguageTag(locale ?: Locale.ENGLISH)
}
