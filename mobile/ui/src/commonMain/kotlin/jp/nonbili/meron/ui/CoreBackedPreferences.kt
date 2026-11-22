// Mirrors mobile preference writes to the native
// cache and the Rust core. Hydration must
// reconcile stored values with edits made during
// startup.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AppPrefsGetParams
import jp.nonbili.meron.shared.AppPrefsSetParams
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.encodeAppPrefValue
import jp.nonbili.meron.shared.parseAppPrefsResponse
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/**
 * Write-through wrapper around the platform preference store.
 *
 * Reads are served entirely from the wrapped store, so they stay synchronous and
 * work before the core is loaded — that is the whole reason the cache exists.
 * Writes go to the cache first (so the UI never waits on the core) and are then
 * mirrored into the core `settings` table, which is authoritative.
 *
 * Only keys in the [mobileSettings] registry are mirrored. Everything else —
 * session state, pending OAuth handshakes — stays device-local by design.
 *
 * Wrapping `AppPreferences` rather than changing every `saveApp*` helper means
 * existing call sites keep working unchanged and cannot forget to write through.
 */
internal class CoreBackedPreferences(
    /** The platform store underneath. Exposed so a hydrate can refill the cache
     *  without echoing every value straight back to the core it came from. */
    val delegate: AppPreferences,
    private val store: PrefStore,
    private val scope: CoroutineScope,
    private val mirror: SettingsMirror,
) : AppPreferences {
    override fun getString(
        key: String,
        default: String,
    ): String = delegate.getString(key, default)

    override fun putString(
        key: String,
        value: String,
    ) {
        delegate.putString(key, value)
        mirrorWrite(key, value)
    }

    override fun getBoolean(
        key: String,
        default: Boolean,
    ): Boolean = delegate.getBoolean(key, default)

    override fun putBoolean(
        key: String,
        value: Boolean,
    ) {
        delegate.putBoolean(key, value)
        mirrorWrite(key, value)
    }

    override fun getInt(
        key: String,
        default: Int,
    ): Int = delegate.getInt(key, default)

    override fun putInt(
        key: String,
        value: Int,
    ) {
        delegate.putInt(key, value)
        mirrorWrite(key, value)
    }

    override fun getStringSet(
        key: String,
        default: Set<String>,
    ): Set<String> = delegate.getStringSet(key, default)

    override fun putStringSet(
        key: String,
        value: Set<String>,
    ) {
        delegate.putStringSet(key, value)
        mirrorWrite(key, value.toList())
    }

    override fun remove(key: String) = delegate.remove(key)

    private fun mirrorWrite(
        key: String,
        value: Any,
    ) {
        val setting = mobileSettingFor(store, key) ?: return
        // Staged synchronously, before the write is even dispatched, so a hydrate
        // racing this write cannot roll the user's change back.
        mirror.stage(setting.settingKey, value)
        scope.launch { mirror.flush() }
    }
}

/**
 * The core half of the write-through, and the hydrate that reconciles the cache
 * against the authoritative table.
 */
internal class SettingsMirror(
    private val core: MeronCore,
    /**
     * Durable home for the pending *key set*, so a write the core never accepted
     * is still known to be outstanding after the process dies. Only keys are
     * journalled: the value itself is already durable in the platform cache, and
     * [recoverPending] reads it back from there.
     */
    private val journal: AppPreferences,
    /** Injectable so tests can run the mirror without a real dispatcher. */
    private val dispatcher: CoroutineDispatcher = ioDispatcher,
    private val coreLoaded: () -> Boolean,
) {
    /**
     * Settings whose cached value is not confirmed into the table yet, holding only the *latest* value per key.
     *
     * An entry means "the cache is ahead of the table": a write is queued, or one never landed because the
     * core was still loading or the call failed. Keeping just the newest value is what makes repeated
     * edits safe — a font-size slider stages dozens of values and only the last one is ever written, so an
     * older one cannot land last and stick.
     */
    private val pending = mutableMapOf<String, Any>()

    /**
     * Serializes the drain. Without it two coroutines could be inside `setPref`
     * for the same key at once and finish in either order, leaving the table
     * holding the older value — the ticket scheme this replaces only ordered the
     * bookkeeping, not the commit.
     */
    private val writeLock = Mutex()

    /** Record a value to mirror. Synchronous, so it cannot race a hydrate. */
    fun stage(
        settingKey: String,
        value: Any,
    ) {
        pending[settingKey] = value
        journalKeys(pending.keys)
    }

    /**
     * Settings the table is known to be behind on, including any this process has
     * not recovered yet — otherwise the first [flush] after a relaunch would
     * report success while journalled writes were still outstanding.
     */
    fun pendingKeys(): Set<String> = pending.keys + journal.getStringSet(PENDING_SETTINGS_PREF, emptySet())

    /**
     * Re-stage writes a previous process could not deliver.
     *
     * Only the keys were journalled; their values come back off the platform cache, which is where the
     * user's choice was durably recorded in the first place. Without this the next hydrate sees no pending
     * keys, treats the older table row as authoritative, and overwrites the newer cached value — the
     * change would silently revert on the launch after the one that made it.
     */
    fun recoverPending(
        app: AppPreferences,
        kanban: AppPreferences,
    ) {
        for (settingKey in journal.getStringSet(PENDING_SETTINGS_PREF, emptySet())) {
            if (settingKey in pending) continue
            val setting = mobileSettingForSettingKey(settingKey) ?: continue
            val prefs = if (setting.store == PrefStore.Kanban) kanban else app
            readCachedSetting(prefs.cacheStore(), setting)?.let { pending[settingKey] = it }
        }
        journalKeys(pending.keys)
    }

    private fun journalKeys(keys: Set<String>) {
        if (keys.isEmpty()) {
            journal.remove(PENDING_SETTINGS_PREF)
        } else {
            journal.putStringSet(PENDING_SETTINGS_PREF, keys.toSet())
        }
    }

    /**
     * Wait for an in-flight write to finish without touching the queue.
     *
     * Used before a restore *attempt*: it stops a straggler from committing on top of the restored rows,
     * while leaving staged values intact in case the attempt turns out to be a passphrase probe, a wrong
     * passphrase, or an unreadable file — none of which change the table, so forgetting local edits there
     * would lose them for nothing.
     */
    suspend fun awaitIdle() = writeLock.withLock { }

    /**
     * Drop queued writes, once a restore has actually replaced the table and
     * those values are known to be superseded.
     */
    suspend fun discardPending() =
        writeLock.withLock {
            pending.clear()
            journalKeys(emptySet())
        }

    /**
     * Write every staged value into the table, newest-only, one at a time.
     *
     * A failure leaves the entry staged: the next flush — or the publish inside
     * [hydrateSettingsFromCore] — retries it, so a write that could not reach the
     * core is repaired rather than silently lost. Returns whether the queue
     * drained completely.
     */
    suspend fun flush(): Boolean {
        if (!coreLoaded()) return pendingKeys().isEmpty()
        writeLock.withLock {
            while (true) {
                val (settingKey, value) = pending.entries.firstOrNull()?.toPair() ?: break
                val stored =
                    runCatching {
                        withContext(dispatcher) {
                            MobileMailCommandClient(core).setPref(
                                AppPrefsSetParams(
                                    key = settingKey,
                                    valueJson = encodeAppPrefValue(value),
                                ),
                            )
                        }
                    }.isSuccess
                if (!stored) break
                // A newer value staged while this one was in flight has to stay
                // queued, or the edit that produced it would be dropped.
                if (pending[settingKey] == value) pending.remove(settingKey)
                journalKeys(pending.keys)
            }
        }
        // Anything left could not be written (a read-only or full database, say).
        // Reported rather than swallowed so a backup does not quietly capture
        // settings the table is still behind on.
        return pendingKeys().isEmpty()
    }

    /** Read every registry key from the authoritative table. */
    suspend fun read(): Map<String, Any> {
        if (!coreLoaded()) return emptyMap()
        return runCatching {
            withContext(dispatcher) {
                parseAppPrefsResponse(
                    MobileMailCommandClient(core).getPrefs(AppPrefsGetParams(mobileSettingKeys)),
                )
            }
        }.getOrDefault(emptyMap())
    }

    /**
     * Publish cached values the table is missing or behind on.
     *
     * Covers two cases at once: the migration path for installs that predate the table being authoritative
     * (absent from `existing`), and the repair path for writes that never landed because the core was down
     * or the call failed (still [pendingKeys]). Anything else is left alone, so this can never overwrite a
     * value the table already agrees on.
     */
    suspend fun publish(
        cached: Map<String, Any>,
        existing: Map<String, Any>,
    ) {
        val stale = pendingKeys()
        for ((settingKey, value) in cached) {
            if (settingKey in existing && settingKey !in stale) continue
            stage(settingKey, value)
        }
        flush()
    }
}

/** The underlying platform store, unwrapping the write-through decorator. */
internal fun AppPreferences.cacheStore(): AppPreferences = (this as? CoreBackedPreferences)?.delegate ?: this

/**
 * Reconcile the platform cache against the authoritative table, once the core is up.
 *
 * Returns the settings whose cached value changed, so the caller can re-seed the UI state holding
 * them.
 *
 * Normally this runs both ways: values the table is behind on are published, and newer values from the
 * table are pulled into the cache. Settings with a write still pending are pulled *from* the cache
 * rather than into it, so a change the user just made is never rolled back by the row it is replacing.
 *
 * [force] is for an explicit restore, where the table has just been overwritten wholesale and is
 * authoritative by definition: nothing is published (that would push pre-restore values back over the
 * restored ones) and every difference is applied, including to settings edited earlier in the session.
 */
internal suspend fun hydrateSettingsFromCore(
    app: AppPreferences,
    kanban: AppPreferences,
    mirror: SettingsMirror,
    force: Boolean = false,
): Map<String, Any> {
    // Writes a previous process staged but never delivered are journalled, not
    // remembered, so bring them back before deciding which side is authoritative.
    mirror.recoverPending(app, kanban)
    val stored = mirror.read()
    val cached = collectCachedSettings(app, kanban)
    // Captured before publishing, because a successful publish clears the very
    // flags that tell us which rows the cache is ahead of. Reading them
    // afterwards would let the stale row we just replaced be pulled back down.
    val unmirrored = if (force) emptySet() else mirror.pendingKeys()
    if (force) {
        // The restore supersedes anything not yet mirrored; dropping the pending
        // set also stops a later publish from undoing it.
        mirror.discardPending()
    } else {
        mirror.publish(cached, stored)
    }
    val changed = stored.filterNot { (key, value) -> cached[key] == value }
    return writeSettingsToCache(app, kanban, changed, skip = unmirrored)
}
