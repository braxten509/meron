// Mailbox fixtures for the desktop state tests.
// Uses the same observable state and command interface as the thread list.
// Fixtures supply synthetic messages, independent of a real mailbox.
import type { Message } from '../../types'
import { type BulkSelectionItem } from '../ui'

export const thread = (overrides: Partial<Message> = {}): Message => ({
  id: 'acc:inbox:thread:1#101',
  account_id: 'acc',
  folder_id: 'inbox',
  thread_id: 'acc:inbox:thread:1',
  from_name: 'Sender',
  from_addr: 'sender@example.com',
  to: 'me@example.com',
  subject: 'Subject',
  preview: '',
  body: '',
  date: Math.floor(Date.parse('2026-06-11T12:00:00Z') / 1000),
  unread: false,
  starred: false,
  has_attachments: false,
  ...overrides,
})

export const nextTick = () => new Promise((resolve) => setTimeout(resolve, 0))

export const bulkItem = (message: Message, overrides: Partial<BulkSelectionItem> = {}): BulkSelectionItem => ({
  key: `test:${message.id}`,
  groupKey: 'test:column',
  threadId: message.thread_id,
  accountId: message.account_id,
  folderId: message.folder_id,
  surface: 'thread-list',
  kind: 'mail',
  unread: message.unread,
  starred: message.starred,
  draft: false,
  trash: false,
  ...overrides,
})
