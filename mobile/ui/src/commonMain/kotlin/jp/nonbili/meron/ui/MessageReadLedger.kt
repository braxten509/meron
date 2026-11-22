// Tracks optimistic read changes for individual
// mobile messages. Completion and failure settle the
// same pending operation against the visible read
// state.

package jp.nonbili.meron.ui

// Read-state bookkeeping for overlapping read/unread writes on the open
// conversation's messages: scroll-driven marks, the whole-thread mark and the
// reader's own toggles.
//
// Each write is an op that sets some messages to a target state. Per message,
// the ledger keeps the last state the core confirmed and the ops still in
// flight. What the UI shows is the newest pending op's target, or the confirmed
// state once nothing is pending. Finishing an op — success or failure — only
// recomputes that, instead of restoring a snapshot the op captured when it
// began: such a snapshot can hold another op's optimistic state that has since
// been turned back, and restoring it would show a state no request produced.
//
// The UI's messages can be replaced behind the ledger's back (a reload hands
// back the core's copy, which lacks pending writes), so `overlay` reapplies
// pending state to whatever is installed, and `begin`/`finish` report the
// resulting state of every message they touch — not only the ones the ledger
// saw change — for the caller to reconcile against what it actually shows.
//
// Ops must finish in the order they began (readMarkMutex serializes them), so a
// later success rightly overrides an earlier one.
internal class MessageReadLedger {
    // `unread` is the state the message should now show. `moved` says whether
    // that differs from what the ledger showed before, which is what a count
    // derived from these messages (the thread card's) should follow.
    data class Change(
        val id: String,
        val folder: String,
        val unread: Boolean,
        val moved: Boolean,
    )

    private class Entry(
        val folder: String,
        var confirmedUnread: Boolean,
        val pending: MutableList<Pair<Long, Boolean>> = mutableListOf(),
    ) {
        fun shown(): Boolean = pending.lastOrNull()?.second ?: confirmedUnread
    }

    private val entries = mutableMapOf<String, Entry>()
    private var nextOp = 0L

    // Start an op setting `messages` (id, folder, currently shown unread) to
    // `unread`. A message with nothing pending is confirmed as what it shows,
    // which is then the core's state. Returns the op id and every touched
    // message's resulting state.
    fun begin(
        messages: List<Triple<String, String, Boolean>>,
        unread: Boolean,
    ): Pair<Long, List<Change>> {
        val op = ++nextOp
        val changes =
            messages.map { (id, folder, shownUnread) ->
                val entry = entries.getOrPut(id) { Entry(folder, shownUnread) }
                val before = entry.shown()
                entry.pending += op to unread
                Change(id, entry.folder, entry.shown(), moved = entry.shown() != before)
            }
        return op to changes
    }

    // Settle an op. Returns every message it touched with its resulting state.
    fun finish(
        op: Long,
        succeeded: Boolean,
    ): List<Change> {
        val changes = mutableListOf<Change>()
        val iterator = entries.iterator()
        while (iterator.hasNext()) {
            val (id, entry) = iterator.next()
            val index = entry.pending.indexOfFirst { it.first == op }
            if (index < 0) continue
            val before = entry.shown()
            val (_, target) = entry.pending.removeAt(index)
            if (succeeded) entry.confirmedUnread = target
            val after = entry.shown()
            changes += Change(id, entry.folder, after, moved = after != before)
            // Nothing in flight: the core holds the truth again, and a reload
            // may replace it, so stop tracking.
            if (entry.pending.isEmpty()) iterator.remove()
        }
        return changes
    }

    // The unread state a message must show while writes to it are in flight,
    // or null when none are.
    fun pendingUnread(id: String): Boolean? = entries[id]?.shown()

    // Reapply pending state to freshly installed items.
    fun <T> overlay(
        items: List<T>,
        id: (T) -> String,
        unread: (T) -> Boolean,
        withUnread: (T, Boolean) -> T,
    ): List<T> {
        if (entries.isEmpty()) return items
        return items.map { item ->
            val pending = pendingUnread(id(item))
            if (pending == null || pending == unread(item)) item else withUnread(item, pending)
        }
    }
}
