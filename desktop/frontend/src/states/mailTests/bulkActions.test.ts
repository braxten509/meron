// Apply commands to a selection of threads.
// Uses the same observable state and command interface as the thread list.
// Fixtures supply synthetic messages, independent of a real mailbox.
import { beforeEach, describe, expect, it } from 'bun:test'
import { kanban$ } from '../kanban'
import { mail$ } from '../mail'
import { bulkMarkSelectedUnread } from '../mailFlags'
import { bulkArchiveSelected, bulkDeleteSelected } from '../mailMoves'
import { settleConfirm, toggleBulkSelection, ui$ } from '../ui'
import { thread, bulkItem } from './fixture'

describe('bulk thread actions', () => {
  const calls: { command: string; payload: unknown }[] = []
  let responses: Record<string, unknown[]> = {}

  beforeEach(() => {
    calls.length = 0
    responses = {
      'mail.markRead': [{ ok: true }],
      'mail.archive': [{ ok: true, moved: 1 }],
      'mail.delete': [{ ok: true, deleted: 1 }],
      'mail.threadList': [{ threads: [] }],
      'mail.folderList': [{ folders: [] }],
    }
    mail$.threads.set([thread({ thread_id: 'acc#inbox#t.MQ' })])
    mail$.messages.set([])
    mail$.folders.set([
      { id: 'inbox', account_id: 'acc', name: 'Inbox', role: 'inbox', unread: 0 },
      { id: 'Trash', account_id: 'acc', name: 'Trash', role: 'trash', unread: 0 },
    ])
    mail$.foldersByAccount.set({
      acc: [
        { id: 'inbox', account_id: 'acc', name: 'Inbox', role: 'inbox', unread: 0 },
        { id: 'Trash', account_id: 'acc', name: 'Trash', role: 'trash', unread: 0 },
      ],
    })
    kanban$.threads.set({})
    ui$.selectedThread.set('acc#inbox#t.MQ')
    ui$.selectedAccount.set('acc')
    ui$.selectedFolder.set('inbox')
    ui$.bulkSelection.set({})
    ui$.bulkAnchorKey.set('')
    ui$.toast.set('')
    ui$.toastTone.set('success')
    ui$.toastUndo.set(null)
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return responses[command]?.shift() ?? {}
          },
        },
      },
    }
  })

  it('dedupes bulk unread by thread id', async () => {
    const first = thread({ id: 'm1', thread_id: 'acc#inbox#t.MQ', unread: false })
    const second = thread({ id: 'm2', thread_id: 'acc#inbox#t.MQ', unread: false })
    mail$.threads.set([first, second])

    await bulkMarkSelectedUnread([bulkItem(first), bulkItem(second)])

    expect(calls.filter((call) => call.command === 'mail.markRead').map((call) => call.payload)).toEqual([
      { thread_id: 'acc#inbox#t.MQ', seen: false },
    ])
    expect(ui$.toast.get()).toBe('Marked unread')
  })

  it('keeps completed bulk archives removed and rolls back the failed thread', async () => {
    const second = thread({ id: 'm2', thread_id: 'acc#inbox#t.NQ' })
    mail$.threads.set([thread({ thread_id: 'acc#inbox#t.MQ' }), second])
    // The failed one gets one more try at the end of the job, and fails again.
    responses['mail.archive'] = [
      { ok: true, moved: 1 },
      { ok: true, moved: 0 },
      { ok: true, moved: 0 },
    ]
    // The list refresh after the job echoes the local list, so a thread the
    // job wrongly dropped (or failed to keep) would show here.
    const invoke = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = (command: string, payload: unknown) =>
      command === 'mail.threadList' ? Promise.resolve({ threads: mail$.threads.get() }) : invoke(command, payload)

    await bulkArchiveSelected(mail$.threads.get().map((item) => bulkItem(item)))

    expect(mail$.threads.get().map((item) => item.thread_id)).toEqual(['acc#inbox#t.NQ'])
    expect(ui$.toastTone.get()).toBe('error')
    expect(ui$.toast.get()).toContain('no matching messages found')
    expect(ui$.toast.get()).toContain('1 of 2')
  })

  it('uses one confirmation for permanent bulk delete', async () => {
    const trash = thread({ id: 'trash', folder_id: 'Trash', thread_id: 'acc#Trash#t.MQ' })
    mail$.threads.set([trash])
    ui$.selectedFolder.set('Trash')
    // A bulk delete goes to the core a group at a time, one answer per thread.
    responses['mail.deleteMany'] = [{ results: [{ ok: true, deleted: 1, permanent: true }] }]
    responses['mail.threadList'] = [{ threads: [] }]

    const pending = bulkDeleteSelected([bulkItem(trash, { trash: true })])
    settleConfirm(true)
    await pending

    expect(calls.filter((call) => call.command === 'mail.deleteMany')).toHaveLength(1)
    expect(calls.find((call) => call.command === 'mail.deleteMany')?.payload).toEqual({
      items: [{ thread_id: 'acc#Trash#t.MQ', folder: 'Trash' }],
    })
    expect(ui$.toast.get()).toBe('Thread deleted')
  })

  it('replaces bulk selection when selecting a different group', () => {
    const first = thread({ id: 'm1', thread_id: 'acc#inbox#t.MQ' })
    const second = thread({ id: 'm2', thread_id: 'acc#work#t.NQ', folder_id: 'Work' })

    toggleBulkSelection(bulkItem(first, { groupKey: 'kanban:inbox' }))
    toggleBulkSelection(bulkItem(second, { groupKey: 'kanban:work' }))

    expect(Object.keys(ui$.bulkSelection.get())).toEqual(['test:m2'])
    expect(ui$.bulkSelection['test:m2'].get()?.groupKey).toBe('kanban:work')
  })
})
