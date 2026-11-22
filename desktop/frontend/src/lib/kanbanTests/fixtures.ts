// Account and message fixtures used by board loading and search tests.
// Each test starts with empty stores and a cleared host bridge.
// Tests override just the fields involved in a column query.
import { accounts$ } from '../../states/accounts'
import { t } from '../i18n'
import { kanban$ } from '../../states/kanban'
import { mail$ } from '../../states/mail'
import type { Account } from '../../types'

export const account = (id: string): Account => ({
  id,
  email: `${id}@example.com`,
  display_name: id,
  provider: 'imap',
  auth_type: 'password',
  imap_host: '',
  imap_port: 993,
  smtp_host: '',
  smtp_port: 465,
  tls: true,
})

export function resetBoardData() {
  accounts$.set([])
  kanban$.threads.set({})
  kanban$.unreadCounts.set({})
  kanban$.loading.set({})
  kanban$.cursors.set({})
  kanban$.accountCursors.set({})
  kanban$.loadingMore.set({})
  kanban$.filters.set({})
  kanban$.globalFilter.set('all')
  mail$.readThreads.set({})
  mail$.foldersByAccount.set({})
  ;(window as any).go = undefined
  ;(window as any).runtime = undefined
}

export const message = (overrides: Partial<import('../../types').Message>): import('../../types').Message => ({
  id: 'm',
  account_id: 'acc1',
  folder_id: 'INBOX',
  thread_id: 't',
  from_name: '',
  from_addr: '',
  to: '',
  subject: '',
  preview: '',
  body: '',
  date: 0,
  unread: false,
  starred: false,
  has_attachments: false,
  ...overrides,
})
