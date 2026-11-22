// Opening a correspondent search from a desktop board.
// Tests start with a thread in a board column and an existing mailbox selection.
// The opened search must carry the account, folder and address from the requested thread.
import { describe, expect, it } from 'bun:test'
import { kanban$, focusKanbanThreadFolder, openCorrespondentMail } from './kanban'
import { ui$ } from './ui'
import { thread$ } from './thread'

describe('openCorrespondentMail', () => {
  it('opens mail search for the address in the conversation account and folder', () => {
    kanban$.activeBoardId.set('')
    ui$.selectedAccount.set('other')
    ui$.selectedFolder.set('Archive')
    ui$.filterMode.set('unread')
    ui$.query.set('old query')
    ui$.selectedThread.set('old-thread')
    ui$.mobilePane.set('conversation')
    thread$.mediaOpen.set(true)

    openCorrespondentMail('acc1', 'INBOX', '  sender@example.com  ')

    expect(ui$.selectedAccount.get()).toBe('acc1')
    expect(ui$.selectedFolder.get()).toBe('INBOX')
    expect(ui$.filterMode.get()).toBe('all')
    expect(ui$.query.get()).toBe('sender@example.com')
    expect(ui$.selectedThread.get()).toBe('')
    expect(ui$.mobilePane.get()).toBe('threads')
    expect(thread$.mediaOpen.get()).toBe(false)
  })

  it('uses the card folder instead of resuming a same-account mail folder', () => {
    const prefsWrites: Array<{ key: string; value: string }> = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: { key: string; value: string }) => {
            if (command === 'app.prefsSet') prefsWrites.push(payload)
            return { ok: true }
          },
        },
      },
    }
    ui$.selectedAccount.set('acc1')
    ui$.selectedFolder.set('Archive')
    kanban$.activeBoardId.set('board')
    focusKanbanThreadFolder('INBOX')
    prefsWrites.length = 0

    openCorrespondentMail('acc1', 'INBOX', 'sender@example.com')

    expect(kanban$.activeBoardId.get()).toBe('')
    expect(ui$.selectedFolder.get()).toBe('INBOX')
    expect(prefsWrites.filter((write) => write.key === 'session_folder')).toEqual([
      { key: 'session_folder', value: 'INBOX' },
    ])
  })

  it('ignores an empty address', () => {
    ui$.query.set('keep me')

    openCorrespondentMail('acc1', 'INBOX', '   ')

    expect(ui$.query.get()).toBe('keep me')
  })
})
