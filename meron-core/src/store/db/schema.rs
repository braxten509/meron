//! Schema for the local mail database.
//!
//! The database opener runs migrations in schema-version order.
//! Existing profiles and fresh installations share the same upgrade path.

pub(in crate::store) const ACCOUNTS_DDL: &str = "
CREATE TABLE IF NOT EXISTS accounts (
  id           TEXT PRIMARY KEY,
  engine       TEXT NOT NULL DEFAULT 'mail',     -- 'mail' | 'rss'
  provider     TEXT NOT NULL DEFAULT 'custom',   -- 'custom' | 'gmail' | 'rss'
  email        TEXT NOT NULL DEFAULT '',
  display_name TEXT NOT NULL DEFAULT '',
  avatar_url   TEXT NOT NULL DEFAULT '',
  sender_name TEXT NOT NULL DEFAULT '',         -- sender name
  config       TEXT NOT NULL DEFAULT '{}',       -- JSON connection metadata (mail)
  prefs        TEXT NOT NULL DEFAULT '{}',       -- user preferences (see AccountPrefs)
  sort_order   INTEGER NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL DEFAULT 0,
  updated_at   INTEGER NOT NULL DEFAULT 0
);
";

pub(in crate::store) const MESSAGES_DDL: &str = "
CREATE TABLE IF NOT EXISTS messages (
  id               INTEGER PRIMARY KEY,   -- stable surrogate rowid; survives VACUUM, is the FTS docid
  account          TEXT NOT NULL,
  folder           TEXT NOT NULL,   -- mail: IMAP folder; rss: subscription id
  msg_id           TEXT NOT NULL,   -- mail: uid as string; rss: item key
  uid              INTEGER NOT NULL DEFAULT 0,   -- mail uid; 0 for rss
  subject          TEXT,
  from_name        TEXT,
  from_addr        TEXT,
  date             INTEGER NOT NULL DEFAULT 0,   -- send time, epoch seconds (0 = unknown)
  seen             INTEGER NOT NULL DEFAULT 0,
  starred          INTEGER NOT NULL DEFAULT 0,
  thread_key       TEXT,
  body             TEXT,
  json             TEXT NOT NULL DEFAULT '{}',   -- JSON catch-all (recipients, body_html, rss fields)
  UNIQUE (account, folder, msg_id)   -- real message identity (mail UID / rss item key)
);
-- (account, folder) lookups are covered by the UNIQUE index prefix.
-- These add the uid ordering / unread / starred access paths the hot queries need.
-- Message lists order by send time (date), with uid as the keyset tiebreaker.
-- `messages_list_idx` keeps uid indexed for the per-uid POINT lookups (mark
-- seen/starred, delete, body-cache write) — those match on uid, not date.
CREATE INDEX IF NOT EXISTS messages_list_idx    ON messages(account, folder, uid);
CREATE INDEX IF NOT EXISTS messages_unread_idx  ON messages(account, folder, date, uid) WHERE seen = 0;
CREATE INDEX IF NOT EXISTS messages_date_idx    ON messages(account, folder, date, uid);
CREATE INDEX IF NOT EXISTS messages_starred_idx ON messages(account, folder, date, uid) WHERE starred <> 0;
-- Cross-account starred view (starred.items) filters only on starred and orders
-- by send time, so it needs a date-leading index independent of account/folder.
CREATE INDEX IF NOT EXISTS messages_starred_all_idx ON messages(date, uid) WHERE starred <> 0;

-- Full-text search over the typed text columns, keyed to messages.id and kept in
-- sync by triggers on the base table so every write path (envelope upsert,
-- body-cache write, prune) is covered. `trigram` does true substring matching
-- (incl. CJK, which has no word breaks); it requires >= 3 codepoints, so shorter
-- queries fall back to LIKE in search_messages. Because `id` is an INTEGER PRIMARY
-- KEY it is stable across VACUUM, so the FTS docid mapping never drifts.
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
  subject, from_name, from_addr, body,
  content='messages', content_rowid='id',
  tokenize='trigram'
);
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, subject, from_name, from_addr, body)
  VALUES (new.id, new.subject, new.from_name, new.from_addr, new.body);
END;
CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, body)
  VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.body);
END;
-- Scoped to the indexed text columns so frequent seen/starred toggles (incl.
-- mark-all-read) don't pointlessly reindex the row in FTS; body-cache writes
-- still touch `body` and so still reindex.
CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE OF subject, from_name, from_addr, body ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, body)
  VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.body);
  INSERT INTO messages_fts(rowid, subject, from_name, from_addr, body)
  VALUES (new.id, new.subject, new.from_name, new.from_addr, new.body);
END;
";

/// Thread-key inheritance (migration v8) looks a message up by its cached
/// Message-ID, and looks up the rows whose thread key names one — both compared
/// case-insensitively, and neither a plain column, since the id lives in the
/// `json` catch-all. Without these expression indexes every reply upsert scanned
/// the account's whole message cache, `json_extract`ing each row. The index
/// expressions must stay character-identical to the ones in
/// `store::resolve_message_thread_key` / `store::reconcile_thread_keys_from`, or
/// SQLite won't match a query to them.
pub(super) const MESSAGES_THREAD_KEY_INDEXES_DDL: &str = "
CREATE INDEX IF NOT EXISTS messages_message_id_idx
  ON messages(account, lower(COALESCE(json_extract(json, '$.message_id'), '')));
CREATE INDEX IF NOT EXISTS messages_thread_key_idx
  ON messages(account, lower(COALESCE(thread_key, '')));
";

/// Recipient search index (migration v6). `To`/`Cc` live in the `json` catch-all,
/// which FTS can't index, so `messages.recipients` mirrors them as flat text
/// (see `store::recipients_index_text`). It gets its own FTS table rather than a
/// fifth column on `messages_fts`: adding a column there would force a full
/// rebuild of the body trigram index — minutes of startup for a large mailbox —
/// while this one indexes a few dozen bytes per row.
pub(super) const MESSAGES_RECIPIENTS_FTS_DDL: &str = "
CREATE VIRTUAL TABLE IF NOT EXISTS messages_recipients_fts USING fts5(
  recipients,
  content='messages', content_rowid='id',
  tokenize='trigram'
);
CREATE TRIGGER IF NOT EXISTS messages_recipients_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_recipients_fts(rowid, recipients) VALUES (new.id, new.recipients);
END;
CREATE TRIGGER IF NOT EXISTS messages_recipients_ad AFTER DELETE ON messages BEGIN
  INSERT INTO messages_recipients_fts(messages_recipients_fts, rowid, recipients)
  VALUES ('delete', old.id, old.recipients);
END;
CREATE TRIGGER IF NOT EXISTS messages_recipients_au AFTER UPDATE OF recipients ON messages BEGIN
  INSERT INTO messages_recipients_fts(messages_recipients_fts, rowid, recipients)
  VALUES ('delete', old.id, old.recipients);
  INSERT INTO messages_recipients_fts(rowid, recipients) VALUES (new.id, new.recipients);
END;
";

pub(super) const OBSERVED_MAIL_IDENTITIES_DDL: &str = "
CREATE TABLE IF NOT EXISTS observed_mail_identities (
  account       TEXT NOT NULL,
  identity      TEXT NOT NULL,
  first_seen_at INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (account, identity)
);
";

pub(super) const MAIL_SEARCH_HITS_DDL: &str = "
CREATE TABLE IF NOT EXISTS mail_search_hits (
  token      TEXT NOT NULL,
  account    TEXT NOT NULL,
  query      TEXT NOT NULL,
  scope      TEXT NOT NULL,
  position   INTEGER NOT NULL,
  folder     TEXT NOT NULL,
  uid        INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (token, position)
);
CREATE INDEX IF NOT EXISTS mail_search_hits_created_idx
  ON mail_search_hits(account, created_at);
";

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS folders (
  account   TEXT NOT NULL,
  name      TEXT NOT NULL,
  delimiter TEXT,
  PRIMARY KEY (account, name)
);
CREATE TABLE IF NOT EXISTS folder_state (
  account        TEXT NOT NULL,
  folder         TEXT NOT NULL,
  uidvalidity    INTEGER,
  uid_next       INTEGER,
  highest_modseq INTEGER,
  PRIMARY KEY (account, folder)
);
CREATE TABLE IF NOT EXISTS subscriptions (
  id            TEXT PRIMARY KEY,
  account       TEXT NOT NULL,
  url           TEXT NOT NULL UNIQUE,
  title         TEXT NOT NULL DEFAULT '',
  site_url      TEXT NOT NULL DEFAULT '',
  feed_title    TEXT NOT NULL DEFAULT '',
  enabled       INTEGER NOT NULL DEFAULT 1,
  last_sync_at  INTEGER NOT NULL DEFAULT 0,
  last_error    TEXT NOT NULL DEFAULT '',
  etag          TEXT NOT NULL DEFAULT '',
  last_modified TEXT NOT NULL DEFAULT '',
  created_at    INTEGER NOT NULL DEFAULT 0,
  updated_at    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS subscriptions_account_idx ON subscriptions(account);
CREATE TABLE IF NOT EXISTS account_secrets (
  account_id TEXT PRIMARY KEY,
  blob       TEXT NOT NULL
);
";

/// Optional Tasks feature. Two tables rather than a JSON blob in `settings`:
/// tasks are user-authored data that grows without bound and is queried by list
/// and by linked thread, which a blob can serve neither cheaply nor safely.
/// `json` is the usual catch-all, so subtasks or recurrence can arrive later
/// without a migration.
pub(in crate::store) const TASKS_DDL: &str = "
CREATE TABLE IF NOT EXISTS task_lists (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL DEFAULT '',
  sort_order INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS tasks (
  id           TEXT PRIMARY KEY,
  list_id      TEXT NOT NULL,
  title        TEXT NOT NULL DEFAULT '',
  notes        TEXT NOT NULL DEFAULT '',
  due_at       INTEGER NOT NULL DEFAULT 0,   -- 0 = no due date
  completed_at INTEGER NOT NULL DEFAULT 0,   -- 0 = not done
  sort_order   INTEGER NOT NULL DEFAULT 0,
  account      TEXT NOT NULL DEFAULT '',     -- mail link; empty for a plain task
  thread_id    TEXT NOT NULL DEFAULT '',
  message_id   TEXT NOT NULL DEFAULT '',
  json         TEXT NOT NULL DEFAULT '{}',
  created_at   INTEGER NOT NULL DEFAULT 0,
  updated_at   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS tasks_list_idx ON tasks(list_id, sort_order);
CREATE INDEX IF NOT EXISTS tasks_thread_idx ON tasks(thread_id);
";
