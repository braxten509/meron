// Defines the persisted desktop settings and the types used by their editors.
// The account and board views share these field shapes and column-size limits.
// Database keys connect the in-memory preferences to the native store.

import { type CustomTheme } from '../lib/themes'
import { type ShortcutOverrides } from '../lib/shortcuts'
import type { ChatWallpaper } from '../types'
import { type SupportedI18nLanguage } from '../lib/i18n'

// Core-persisted user settings map to the `settings` DB table; native tray
// style is loaded separately through tray.getStyle. Each core-backed
// field is one row, keyed by `DB_KEY`. Persistence is centralized here (a single
// root listener) so individual fields never wire up their own save logic.

/** How the quick reply composer sends: bare Enter, or Cmd/Ctrl+Enter. */
export type SendShortcut = 'enter' | 'mod_enter'

/**
 * How a thread's messages are laid out.
 * 'chat': left/right chat bubbles, every message expanded.
 * 'traditional': full-width stacked messages, collapsed to a one-line summary
 * except the newest and the unread ones (the classic mail-client reading view).
 */
export type ConversationLayout = 'chat' | 'traditional'

export type KanbanBoardColumn = {
  accountId: string
  folderId: string
  /** A saved search: the column lists only this folder's matches for it. */
  query?: string
}

export type KanbanBoard = {
  id: string
  name: string
  columns: KanbanBoardColumn[]
  /** Custom rail/header image, e.g. "/media/avatars/kb-…/<uuid>.png". Unset = Columns3 tile. */
  avatarUrl?: string
  /** Background behind the board's columns. Unset = plain theme surface. */
  wallpaper?: ChatWallpaper | null
  /** Show the board's unread count on its rail tile. Unset = off. */
  showUnreadBadge?: boolean
}

/** Proxy transport, or 'off' for direct connections. */
export type ProxyMode = 'off' | 'http' | 'socks5'

/**
 * Proxy endpoint, shared by the app-wide setting and the per-account override.
 * An empty `username` means the proxy needs no authentication.
 */
export type ProxySettings = {
  mode: ProxyMode
  host: string
  /** 0 while the field is empty; the core treats that as "no proxy". */
  port: number
  username: string
  password: string
}

export const EMPTY_PROXY: ProxySettings = {
  mode: 'off',
  host: '',
  port: 0,
  username: '',
  password: '',
}

export type Settings = {
  /**
   * Active built-in or custom theme id while not following the system. The
   * theme's appearance controls light/dark mode.
   */
  themeId: string
  /** Whether the theme switches between `lightThemeId` and `darkThemeId` with the OS appearance. */
  themeFollowSystem: boolean
  /** Theme painted while following the system and the OS is light. */
  lightThemeId: string
  /** Theme painted while following the system and the OS is dark. */
  darkThemeId: string
  /** User-created themes (see lib/themes.ts). */
  customThemes: CustomTheme[]
  /** Interface font: '' for Inter, a lib/fonts option id, or a typed family name. */
  fontFamily: string
  /** Font for message bodies; '' follows the interface font. */
  messageFontFamily: string
  /** App-wide text size, in percent of the default (see lib/fonts). */
  fontScale: number
  /** Message body text size, in percent, applied on top of `fontScale`. */
  messageFontScale: number
  showRealAvatars: boolean
  /** Whether a message that brings its own light design is drawn inverted under
   *  a dark theme, instead of as a light card (see `darkensCanvas`). */
  darkMessageBodies: boolean
  /** Whether a bubble shrinks a table too wide for it to fit, instead of scrolling it. */
  autoFitMessages: boolean
  /** Whether reader tabs put reply, forward and delete in a bar at the bottom. */
  readerBottomActions: boolean
  /** Whether to overlay an inbox unread-count badge on side navigation account avatars. */
  showUnreadAccountBadge: boolean
  sendShortcut: SendShortcut
  /** Chat bubbles or the traditional stacked reading view (desktop only). */
  conversationLayout: ConversationLayout
  /** Whether chat bubbles grow to fit long bodies instead of scrolling inside a
   *  capped box, so only the conversation scrolls (desktop only). */
  chatFullMessages: boolean
  /** Whether native spell checking is requested in composer prose fields. */
  spellCheck: boolean
  /** Whether replies carry the message they answer, quoted under the reply text. */
  quoteReplies: boolean
  /**
   * App-wide signature HTML, inserted into new messages and replies. Accounts
   * follow this unless they carry their own override (see Account.signature).
   * Empty means "no signature".
   */
  signature: string
  /**
   * Sender addresses (bare, lowercased) whose remote content always loads,
   * whatever an account's own "load remote images" toggle says. Grown from the
   * "Always allow from …" action on a message with blocked remote content.
   */
  remoteImageSenders: string[]
  /** Ordered user-created kanban boards. */
  kanbanBoards: KanbanBoard[]
  threadListWidth: number
  kanbanPaneWidth: number
  /** Pixel width used by every expanded kanban column. */
  kanbanColumnWidth: number
  /**
   * Whether the board keeps its horizontal scroll position while a card or
   * column is being dragged, instead of auto-scrolling toward the pointer.
   */
  kanbanLockScroll: boolean
  /** kanbanColumnKey -> whether the column is collapsed to a vertical bar. */
  kanbanMinimizedColumns: Record<string, boolean>
  /** Account ids hidden from the desktop side navigation. */
  hiddenSideNavAccounts: string[]
  /** Whether the synthetic unified inbox appears in the desktop side navigation. */
  showUnifiedInboxInSideNav: boolean
  /** Whether the full calendar has an item in the desktop side navigation. */
  showCalendarInSideNav: boolean
  /**
   * Whether the optional Tasks surface is available. Off by default: an upgrade
   * shouldn't grow a rail entry nobody asked for. The tasks themselves live in
   * the core's own tables, not here — this is only whether the app shows them.
   */
  tasksEnabled: boolean
  /** Whether the optional calendar is available, like [tasksEnabled]. */
  calendarEnabled: boolean
  /** Default alert for timed events, minutes before start or NO_ALERT. Read by the core. */
  reminderLeadMinutes: number
  /** Default alert for all-day events, days before at 09:00 or NO_ALERT. Read by the core. */
  reminderAllDayDays: number
  /**
   * Whether the window's close button hides Meron to the tray instead of
   * quitting (Linux and Windows; macOS keeps its own close/quit split).
   */
  closeToTray: boolean
  monochromeTrayIcon: boolean
  /** Whether to poll for new releases in the background (see states/update.ts). */
  autoUpdateCheck: boolean
  /** Version whose update banner the user dismissed, so it doesn't nag. */
  dismissedUpdateVersion: string | null
  language: SupportedI18nLanguage | null
  /** Rebound keyboard shortcuts, keyed by shortcut id (see lib/shortcuts.ts). */
  shortcutOverrides: ShortcutOverrides
  /**
   * App-wide proxy for mail sockets, feed fetches and OAuth calls. Accounts
   * follow this unless they carry their own override (see AccountProxyCard).
   */
  proxy: ProxySettings
}

export const KANBAN_COLUMN_DEFAULT_WIDTH = 360

export const KANBAN_COLUMN_MIN_WIDTH = 240

export const KANBAN_COLUMN_MAX_WIDTH = 700

// The field <-> DB row mapping. Add a field here and it persists automatically;
// nothing else needs to change.
export const DB_KEY = {
  themeId: 'theme_id',
  themeFollowSystem: 'theme_follow_system',
  lightThemeId: 'light_theme_id',
  darkThemeId: 'dark_theme_id',
  customThemes: 'custom_themes',
  fontFamily: 'font_family',
  messageFontFamily: 'message_font_family',
  fontScale: 'font_scale',
  messageFontScale: 'message_font_scale',
  showRealAvatars: 'show_real_avatars',
  darkMessageBodies: 'dark_message_bodies',
  autoFitMessages: 'auto_fit_messages',
  readerBottomActions: 'reader_bottom_actions',
  showUnreadAccountBadge: 'show_unread_account_badge',
  sendShortcut: 'send_shortcut',
  conversationLayout: 'conversation_layout',
  chatFullMessages: 'chat_full_messages',
  spellCheck: 'spell_check',
  quoteReplies: 'quote_replies',
  signature: 'signature',
  remoteImageSenders: 'remote_image_senders',
  kanbanBoards: 'kanban_boards',
  threadListWidth: 'thread_list_width',
  kanbanPaneWidth: 'kanban_pane_width',
  kanbanColumnWidth: 'kanban_column_width',
  kanbanLockScroll: 'kanban_lock_scroll',
  kanbanMinimizedColumns: 'kanban_minimized_columns',
  hiddenSideNavAccounts: 'hidden_sidenav_accounts',
  showUnifiedInboxInSideNav: 'show_unified_inbox_in_sidenav',
  showCalendarInSideNav: 'show_calendar_in_sidenav',
  tasksEnabled: 'tasks_enabled',
  calendarEnabled: 'calendar_enabled',
  reminderLeadMinutes: 'reminder_lead_minutes',
  reminderAllDayDays: 'reminder_all_day_days',
  closeToTray: 'close_to_tray',
  autoUpdateCheck: 'auto_update_check',
  dismissedUpdateVersion: 'dismissed_update_version',
  language: 'language',
  shortcutOverrides: 'shortcut_overrides',
  proxy: 'proxy',
} satisfies Record<Exclude<keyof Settings, 'monochromeTrayIcon'>, string>

/** Keys to request from `app.prefsGet` on boot. */
export const SETTINGS_DB_KEYS = Object.values(DB_KEY)
