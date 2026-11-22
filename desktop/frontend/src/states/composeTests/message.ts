import type { Message } from '../../types'

export const message = (overrides: Partial<Message> = {}): Message => ({
  id: 'm1',
  account_id: 'acc-notification',
  folder_id: 'inbox',
  thread_id: 't-notification',
  from_name: 'Sender',
  from_addr: 'sender@example.com',
  to: 'me@example.com',
  subject: 'Notification thread',
  preview: '',
  body: '',
  date: Math.floor(Date.parse('2026-06-11T12:00:00Z') / 1000),
  unread: true,
  starred: false,
  has_attachments: false,
  ...overrides,
})
