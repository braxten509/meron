// A message fixture for board state tests.
// It identifies one unread message in an account inbox.
// Callers can override its thread, folder and flags for each state transition.
import type { Message } from '../../types'

export const message = (overrides: Partial<Message> = {}): Message => ({
  id: 'acc1:INBOX:t1#1',
  account_id: 'acc1',
  folder_id: 'INBOX',
  thread_id: 'acc1#INBOX#t1',
  from_name: 'Sender',
  from_addr: 'sender@example.com',
  to: 'me@example.com',
  subject: 'Subject',
  preview: '',
  body: '',
  date: 1,
  unread: true,
  starred: false,
  has_attachments: false,
  ...overrides,
})
