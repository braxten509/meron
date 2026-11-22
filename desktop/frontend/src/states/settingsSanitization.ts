// Normalizes settings as they enter desktop state.
// Board columns, proxy configuration and sender addresses have distinct validation rules.
// The settings loader and editing actions reuse the same normalization functions.

import { sanitizeChatWallpaper } from '../lib/wallpapers'
import {
  type KanbanBoard,
  type KanbanBoardColumn,
  KANBAN_COLUMN_MAX_WIDTH,
  KANBAN_COLUMN_MIN_WIDTH,
  type ProxySettings,
} from './settingsModel'

/** A saved search query as a column stores it: one line, trimmed. The column key
 * joins its parts with newlines, so a query must not carry one. */
export function normalizeKanbanColumnQuery(query: string): string {
  return query.replace(/\s+/g, ' ').trim()
}

export function sanitizeKanbanBoards(raw: unknown): KanbanBoard[] | null {
  if (!Array.isArray(raw)) return null
  const out: KanbanBoard[] = []
  const seen = new Set<string>()
  for (const item of raw) {
    if (!item || typeof item !== 'object' || Array.isArray(item)) continue
    const record = item as Record<string, unknown>
    if (typeof record.id !== 'string' || !record.id || seen.has(record.id)) continue
    const name = typeof record.name === 'string' && record.name.trim() ? record.name.trim() : 'Kanban board'
    const columns = Array.isArray(record.columns)
      ? record.columns.flatMap((column): KanbanBoardColumn[] => {
          if (!column || typeof column !== 'object' || Array.isArray(column)) return []
          const col = column as Record<string, unknown>
          if (typeof col.accountId !== 'string' || typeof col.folderId !== 'string') return []
          if (!col.accountId || !col.folderId) return []
          const query = typeof col.query === 'string' ? normalizeKanbanColumnQuery(col.query) : ''
          return [
            query
              ? { accountId: col.accountId, folderId: col.folderId, query }
              : { accountId: col.accountId, folderId: col.folderId },
          ]
        })
      : []
    seen.add(record.id)
    const board: KanbanBoard = { id: record.id, name, columns }
    // Board images are app-managed uploads; only paths under /media/avatars/
    // (written by account.writeAvatarFile) are accepted.
    if (typeof record.avatarUrl === 'string' && record.avatarUrl.startsWith('/media/avatars/')) {
      board.avatarUrl = record.avatarUrl
    }
    const wallpaper = sanitizeChatWallpaper(record.wallpaper)
    if (wallpaper) board.wallpaper = wallpaper
    if (record.showUnreadBadge === true) board.showUnreadBadge = true
    out.push(board)
  }
  return out
}

export function clampKanbanColumnWidth(width: number): number {
  return Math.round(Math.min(KANBAN_COLUMN_MAX_WIDTH, Math.max(KANBAN_COLUMN_MIN_WIDTH, width)))
}

export function sanitizeBooleanMap(raw: unknown): Record<string, boolean> | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const out: Record<string, boolean> = {}
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    if (typeof value === 'boolean') out[key] = value
  }
  return out
}

/**
 * Coerce a stored or user-entered proxy into the canonical shape. Returns null
 * for anything unrecognizable so hydration leaves the current value alone.
 */
export function sanitizeProxy(raw: unknown): ProxySettings | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const v = raw as Record<string, unknown>
  const mode = v.mode
  if (mode !== 'off' && mode !== 'http' && mode !== 'socks5') return null
  const port = typeof v.port === 'number' && Number.isFinite(v.port) ? Math.trunc(v.port) : 0
  return {
    mode,
    host: typeof v.host === 'string' ? v.host.trim() : '',
    port: port > 0 && port <= 65535 ? port : 0,
    username: typeof v.username === 'string' ? v.username : '',
    password: typeof v.password === 'string' ? v.password : '',
  }
}

/** Whether a proxy is configured well enough to actually be used. */
export function isProxyUsable(proxy: ProxySettings): boolean {
  return proxy.mode !== 'off' && proxy.host.trim() !== '' && proxy.port > 0
}

export function sanitizeStringArray(raw: unknown): string[] | null {
  if (!Array.isArray(raw)) return null
  const out: string[] = []
  const seen = new Set<string>()
  for (const value of raw) {
    if (typeof value !== 'string' || !value || seen.has(value)) continue
    seen.add(value)
    out.push(value)
  }
  return out
}

// The form the allowlist stores and compares: bare address, lowercased. Mirrors
// `normalize_sender` in the sidecar so both ends agree on membership.
export function normalizeSenderAddr(addr: string): string {
  const trimmed = addr.trim()
  const open = trimmed.lastIndexOf('<')
  const close = trimmed.lastIndexOf('>')
  const bare = open >= 0 && close > open ? trimmed.slice(open + 1, close) : trimmed
  return bare.trim().toLowerCase()
}
