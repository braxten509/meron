// Folder identities, unread badges and refresh events in board columns.
// Tests supply account folders and aggregate unread state for the board.
// Snapshot and event cases determine which columns need fresh data.
import { beforeEach, describe, expect, it } from 'bun:test'
import { t } from '../i18n'
import { kanban$ } from '../../states/kanban'
import { mail$ } from '../../states/mail'
import { settings$ } from '../../states/settings'
import type { Folder } from '../../types'
import {
  accountLabel,
  folderLabel,
  folderMatches,
  isUnifiedStarredColumn,
  isRSSAccount,
  kanbanColumnMatchesMailEvent,
  kanbanColumnUnreadCount,
  mergeLabelFolders,
  nextFoldersSnapshot,
  searchColumnLabel,
  subscribeKanbanMailReloads,
} from '../kanbanData'
import { account, resetBoardData, message } from './fixtures'

beforeEach(resetBoardData)

describe('folderMatches', () => {
  it('matches the inbox role case-insensitively', () => {
    expect(folderMatches('inbox', 'INBOX')).toBe(true)
    expect(folderMatches('INBOX', 'inbox')).toBe(true)
  })

  it('matches other folders exactly', () => {
    expect(folderMatches('Archive', 'Archive')).toBe(true)
    expect(folderMatches('Archive', 'archive')).toBe(false)
  })

  it('is false when nothing synced', () => {
    expect(folderMatches('inbox', undefined)).toBe(false)
  })
})

describe('kanbanColumnUnreadCount', () => {
  it('uses the unread total returned with a mail column page', () => {
    expect(
      kanbanColumnUnreadCount({ accountId: 'acc1', folderId: 'inbox' }, 137, [
        message({ unread: true, unread_count: 1 }),
        message({ id: 'm2', thread_id: 't2', unread: true, unread_count: 1 }),
      ]),
    ).toBe(137)
  })

  it('trusts a zero total returned with the page', () => {
    expect(
      kanbanColumnUnreadCount({ accountId: 'acc1', folderId: 'inbox' }, 0, [
        message({ unread: true, unread_count: 1 }),
      ]),
    ).toBe(0)
  })

  it('falls back to loaded message totals before the first page returns', () => {
    expect(
      kanbanColumnUnreadCount({ accountId: 'acc1', folderId: 'inbox' }, undefined, [
        message({ unread: true, unread_count: 3 }),
      ]),
    ).toBe(3)
  })

  it('counts a saved search from its loaded matches, not the folder total', () => {
    expect(
      kanbanColumnUnreadCount({ accountId: 'acc1', folderId: 'inbox', query: 'invoice' }, 137, [
        message({ unread: true, unread_count: 2 }),
      ]),
    ).toBe(2)
  })

  it('always derives unified starred from its loaded items', () => {
    expect(
      kanbanColumnUnreadCount({ accountId: 'unified', folderId: 'starred' }, 99, [
        message({ unread: true, unread_count: 3 }),
        message({ id: 'm2', thread_id: 't2', unread: false, unread_count: 0 }),
      ]),
    ).toBe(3)
  })
})

describe('isRSSAccount', () => {
  const accounts = [
    { id: 'acc1', provider: 'rss', auth_type: '' },
    { id: 'acc2', provider: 'gmail', auth_type: 'oauth' },
  ]

  it('detects rss provider and rss- prefix', () => {
    expect(isRSSAccount('acc1', accounts)).toBe(true)
    expect(isRSSAccount('rss-anything', [])).toBe(true)
  })

  it('is false for mail accounts', () => {
    expect(isRSSAccount('acc2', accounts)).toBe(false)
  })
})

describe('labels', () => {
  const t = (key: string) =>
    ({
      'kanban.columns.unifiedInbox': 'Unified inbox',
      'kanban.columns.unifiedStarred': 'Unified starred',
      'kanban.columns.unifiedSent': 'Unified sent',
      'kanban.columns.unifiedDrafts': 'Unified drafts',
      'kanban.columns.unifiedArchive': 'Unified archive',
      'kanban.columns.unifiedJunk': 'Unified junk',
      'kanban.columns.unifiedTrash': 'Unified trash',
      'folders.roles.inbox': 'Inbox',
    })[key] ?? key

  const accounts = [
    { id: 'acc1', email: 'a@x.com', display_name: 'Alice' },
    { id: 'acc2', email: 'b@x.com', display_name: '' },
  ]

  it('accountLabel prefers display name, then email, then id', () => {
    expect(accountLabel('unified', accounts)).toBe('Unified')
    expect(accountLabel('acc1', accounts)).toBe('Alice')
    expect(accountLabel('acc2', accounts)).toBe('b@x.com')
    expect(accountLabel('missing', accounts)).toBe('missing')
  })

  it('folderLabel names unified, inbox, and stored folders', () => {
    const folders = [{ account_id: 'acc1', id: 'f1', name: 'Receipts' } as Folder]
    const accs = [account('acc1')]
    expect(folderLabel({ accountId: 'unified', folderId: 'inbox' }, folders, accs, t)).toBe('Unified inbox')
    expect(isUnifiedStarredColumn({ accountId: 'unified', folderId: 'starred' })).toBe(true)
    expect(folderLabel({ accountId: 'unified', folderId: 'starred' }, folders, accs, t)).toBe('Unified starred')
    expect(folderLabel({ accountId: 'acc1', folderId: 'INBOX' }, folders, accs, t)).toBe('Inbox')
    expect(folderLabel({ accountId: 'acc1', folderId: 'f1' }, folders, accs, t)).toBe('Receipts')
    expect(folderLabel({ accountId: 'acc1', folderId: 'f2' }, folders, accs, t)).toBe('f2')
  })

  it('folderLabel names local folders without an inbox after the account', () => {
    const local = { ...account('local'), display_name: 'Local Folders' }
    const folders = [{ account_id: 'local', id: 'INBOX', name: 'INBOX', delimiter: '/' } as Folder]
    expect(folderLabel({ accountId: 'local', folderId: 'inbox' }, [], [local], t)).toBe('Local Folders')
    // A local folder the user named "inbox" is stored as INBOX and is real.
    expect(folderLabel({ accountId: 'local', folderId: 'INBOX' }, folders, [local], t)).toBe('INBOX')
  })

  it('folderLabel names a nested folder by its last path segment', () => {
    const folders = [
      { account_id: 'acc1', id: '[Gmail]/Now', name: '[Gmail]/Now', delimiter: '/' } as Folder,
      { account_id: 'acc1', id: 'INBOX.Later', name: 'INBOX.Later', delimiter: '.' } as Folder,
    ]
    const accs = [account('acc1')]
    expect(folderLabel({ accountId: 'acc1', folderId: '[Gmail]/Now' }, folders, accs, t)).toBe('Now')
    expect(folderLabel({ accountId: 'acc1', folderId: 'INBOX.Later' }, folders, accs, t)).toBe('Later')
    expect(searchColumnLabel({ accountId: 'acc1', folderId: '[Gmail]/Now' }, folders, accs)).toBe('acc1 / Now')
  })

  it('folderLabel keeps an RSS feed title whole', () => {
    // Feed titles are not IMAP paths: a "/" in one is part of the name.
    const folders = [{ account_id: 'rss-1', id: 'feed1', name: 'News / Tech' } as Folder]
    expect(folderLabel({ accountId: 'rss-1', folderId: 'feed1' }, folders, [], t)).toBe('News / Tech')
  })

  it('searchColumnLabel names a unified column by its role', () => {
    const accs = [account('acc1')]
    expect(searchColumnLabel({ accountId: 'unified', folderId: 'sent' }, [], accs, t)).toBe('Unified sent')
    expect(searchColumnLabel({ accountId: 'unified', folderId: 'starred' }, [], accs, t)).toBe('Unified starred')
  })

  it('matches unified columns to each account own folder in sync events', () => {
    const byAccount = {
      acc1: [{ account_id: 'acc1', id: '[Gmail]/Sent Mail', name: 'Sent', role: 'sent' } as Folder],
    }
    expect(
      kanbanColumnMatchesMailEvent({ accountId: 'unified', folderId: 'sent' }, 'acc1', '[Gmail]/Sent Mail', byAccount),
    ).toBe(true)
    expect(kanbanColumnMatchesMailEvent({ accountId: 'unified', folderId: 'sent' }, 'acc1', 'INBOX', byAccount)).toBe(
      false,
    )
    expect(
      kanbanColumnMatchesMailEvent({ accountId: 'unified', folderId: 'inbox' }, 'acc1', undefined, byAccount),
    ).toBe(true)
    expect(
      kanbanColumnMatchesMailEvent({ accountId: 'unified', folderId: 'starred' }, 'acc1', 'INBOX', byAccount),
    ).toBe(true)
  })

  it('mergeLabelFolders flattens per-account folders after the base list', () => {
    const base = [{ account_id: 'acc1', id: 'f1', name: 'A' } as Folder]
    const byAccount = { acc2: [{ account_id: 'acc2', id: 'f2', name: 'B' } as Folder] }
    expect(mergeLabelFolders(base, byAccount).map((f) => f.id)).toEqual(['f1', 'f2'])
  })
})

describe('nextFoldersSnapshot', () => {
  const folder = (id: string): Folder => ({ account_id: 'acc1', id, name: id }) as Folder

  it('keeps the previous object so memoized derivations stay stable', () => {
    const current = { acc1: [folder('f1')] }
    const first = nextFoldersSnapshot({}, current)
    expect(nextFoldersSnapshot(first, current)).toBe(first)
  })

  // The bug this exists for: Legend-State replaces one account's array in place on
  // the *same* record object, so a picker memoized on that object never saw a
  // folder created in the app until the next restart.
  it('mints a new object when an account list is replaced in place', () => {
    const live: Record<string, Folder[]> = { acc1: [folder('f1')] }
    const first = nextFoldersSnapshot({}, live)
    live.acc1 = [folder('f1'), folder('created')]
    const second = nextFoldersSnapshot(first, live)
    expect(second).not.toBe(first)
    expect(second.acc1.map((item) => item.id)).toEqual(['f1', 'created'])
    expect(first.acc1.map((item) => item.id)).toEqual(['f1'])
  })

  it('mints a new object when an account appears or disappears', () => {
    const live: Record<string, Folder[]> = { acc1: [folder('f1')] }
    const first = nextFoldersSnapshot({}, live)
    live.acc2 = [folder('f2')]
    const second = nextFoldersSnapshot(first, live)
    expect(second).not.toBe(first)
    delete live.acc1
    expect(nextFoldersSnapshot(second, live)).not.toBe(second)
  })
})

describe('subscribeKanbanMailReloads', () => {
  const setup = () => {
    const handlers = new Map<string, (detail: { account?: string; folder?: string }) => void>()
    const calls: { command: string; payload: any }[] = []
    settings$.kanbanBoards.set([
      { id: 'board-a', name: 'A', columns: [{ accountId: 'acc1', folderId: 'INBOX' }] },
      { id: 'board-b', name: 'B', columns: [{ accountId: 'acc2', folderId: 'Archive' }] },
    ])
    kanban$.activeBoardId.set('board-a')
    kanban$.searchQuery.set('')
    kanban$.searchScope.set('all')
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: any) => {
            calls.push({ command, payload })
            return { threads: [], next_cursor: '', folder_unread: 0 }
          },
        },
      },
    }
    const unsubscribe = subscribeKanbanMailReloads((name, callback) => {
      handlers.set(name, callback)
      return () => handlers.delete(name)
    })
    const cleanup = () => {
      unsubscribe()
      kanban$.activeBoardId.set('')
    }
    return { calls, cleanup, handlers }
  }

  it('keeps a queued new-mail reload alive when the folder cache changes', async () => {
    const { calls, cleanup, handlers } = setup()
    try {
      handlers.get('mail.newMessages')?.({ account: 'acc1', folder: 'inbox' })
      mail$.foldersByAccount.acc1.set([{ account_id: 'acc1', id: 'INBOX', name: 'Inbox', role: 'inbox', unread: 1 }])
      await Bun.sleep(275)

      expect(calls.filter((call) => call.command === 'mail.threadList').map((call) => call.payload)).toEqual([
        {
          account_id: 'acc1',
          folder_id: 'INBOX',
          query: '',
          filter: 'all',
          attachments: false,
          refresh: false,
          limit: 50,
        },
      ])
    } finally {
      cleanup()
    }
  })

  it("reloads the account's columns for a sent copy, which names no folder", async () => {
    const { calls, cleanup, handlers } = setup()
    try {
      handlers.get('mail.sentCopyCached')?.({ account: 'acc1' })
      await Bun.sleep(275)

      expect(calls.filter((call) => call.command === 'mail.threadList').map((call) => call.payload)).toEqual([
        {
          account_id: 'acc1',
          folder_id: 'INBOX',
          query: '',
          filter: 'all',
          attachments: false,
          refresh: false,
          limit: 50,
        },
      ])
    } finally {
      cleanup()
    }
  })

  it('leaves other accounts alone when a sent copy is cached', async () => {
    const { calls, cleanup, handlers } = setup()
    try {
      handlers.get('mail.sentCopyCached')?.({ account: 'acc2' })
      await Bun.sleep(275)

      expect(calls.filter((call) => call.command === 'mail.threadList')).toEqual([])
    } finally {
      cleanup()
    }
  })

  it('resolves the active board and folder roles when each event arrives', async () => {
    const { calls, cleanup, handlers } = setup()
    try {
      kanban$.activeBoardId.set('board-b')
      settings$.kanbanBoards.set([
        { id: 'board-a', name: 'A', columns: [{ accountId: 'acc1', folderId: 'INBOX' }] },
        { id: 'board-b', name: 'B', columns: [{ accountId: 'unified', folderId: 'archive' }] },
      ])
      mail$.foldersByAccount.acc2.set([
        { account_id: 'acc2', id: 'All Mail', name: 'Archive', role: 'archive', unread: 0 },
      ])

      handlers.get('mail.synced')?.({ account: 'acc2', folder: 'All Mail' })
      await Bun.sleep(275)

      expect(calls.filter((call) => call.command === 'mail.threadList').map((call) => call.payload)).toEqual([
        {
          account_id: 'unified',
          folder_id: 'archive',
          folder_role: 'archive',
          query: '',
          filter: 'all',
          attachments: false,
          refresh: false,
          limit: 50,
        },
      ])
    } finally {
      cleanup()
    }
  })
})
