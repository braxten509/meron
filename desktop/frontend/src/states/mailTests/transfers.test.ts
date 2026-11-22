// Move, copy and undo thread transfers.
// Uses the same observable state and command interface as the thread list.
// Fixtures supply synthetic messages, independent of a real mailbox.
import { beforeEach, describe, expect, it } from 'bun:test'
import { kanban$ } from '../kanban'
import { mail$ } from '../mail'
import { archiveThread, copyThreadToFolder, deleteThread, moveThreadToFolder } from '../mailMoves'
import { runToastUndo, settleConfirm, ui$ } from '../ui'
import { thread, nextTick } from './fixture'

describe('moveThreadToFolder undo', () => {
  const calls: { command: string; payload: unknown }[] = []
  let responses: Record<string, unknown[]> = {}

  beforeEach(() => {
    calls.length = 0
    responses = {
      'mail.move': [
        { ok: true, moved: 1, target_uids: [5] },
        { ok: true, moved: 1 },
      ],
      'mail.threadList': [{ threads: [] }, { threads: [thread({ thread_id: 'acc#inbox#t.MQ' })] }],
    }
    mail$.threads.set([thread({ thread_id: 'acc#inbox#t.MQ' })])
    mail$.messages.set([])
    kanban$.threads.set({})
    ui$.selectedThread.set('acc#inbox#t.MQ')
    ui$.selectedAccount.set('acc')
    ui$.selectedFolder.set('inbox')
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

  it('offers undo for a successful folder move', async () => {
    await moveThreadToFolder('acc#inbox#t.MQ', 'Work')

    expect(calls.find((call) => call.command === 'mail.move')?.payload).toMatchObject({
      thread_id: 'acc#inbox#t.MQ',
      target_folder_id: 'Work',
    })
    expect(ui$.toast.get()).toBe('Thread moved')
    expect(ui$.toastUndo.peek()).toBeTruthy()

    runToastUndo()
    await nextTick()

    expect(calls.filter((call) => call.command === 'mail.move')[1]?.payload).toMatchObject({
      thread_id: 'acc#Work#t.MQ',
      target_folder_id: 'inbox',
      message_ids: ['5'],
    })
    expect(ui$.toast.get()).toBe('Thread moved')
    expect(ui$.toastUndo.peek()).toBeNull()
  })
})

describe('copyThreadToFolder', () => {
  const calls: { command: string; payload: unknown }[] = []
  let responses: Record<string, unknown[]> = {}

  beforeEach(() => {
    calls.length = 0
    responses = {
      'mail.copy': [{ ok: true, copied: 1 }],
      'mail.threadList': [{ threads: [thread({ thread_id: 'acc#inbox#t.MQ' })] }],
      'mail.folderList': [{ folders: [] }],
    }
    mail$.threads.set([thread({ thread_id: 'acc#inbox#t.MQ' })])
    mail$.messages.set([])
    kanban$.threads.set({})
    ui$.selectedThread.set('acc#inbox#t.MQ')
    ui$.selectedAccount.set('acc')
    ui$.selectedFolder.set('inbox')
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

  it('copies a thread to the requested account and folder', async () => {
    await copyThreadToFolder('acc#inbox#t.MQ', 'acc2', 'Archive')

    expect(calls.find((call) => call.command === 'mail.copy')?.payload).toMatchObject({
      thread_id: 'acc#inbox#t.MQ',
      target_account_id: 'acc2',
      target_folder_id: 'Archive',
    })
    expect(calls.some((call) => call.command === 'mail.delete')).toBe(false)
    expect(ui$.toast.get()).toBe('Thread copied')
  })

  it('reports an error when copy affects no messages', async () => {
    responses['mail.copy'] = [{ ok: true, copied: 0 }]

    await copyThreadToFolder('acc#inbox#t.MQ', 'acc2', 'Archive')

    expect(ui$.toastTone.get()).toBe('error')
    expect(ui$.toast.get()).toBe('Copy failed: no matching messages found')
  })
})

describe('deleteThread trash undo', () => {
  const calls: { command: string; payload: unknown }[] = []
  let responses: Record<string, unknown[]> = {}

  beforeEach(() => {
    calls.length = 0
    responses = {
      'mail.delete': [{ ok: true, deleted: 1, trash: 'Trash', thread_id: 'acc#Trash#t.MQ', target_uids: [42] }],
      'mail.move': [{ ok: true, moved: 1 }],
      'mail.threadList': [{ threads: [] }, { threads: [thread({ thread_id: 'acc#inbox#t.MQ' })] }],
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

  it('offers undo for moving a thread to Trash', async () => {
    await deleteThread('acc#inbox#t.MQ')

    expect(ui$.toast.get()).toBe('Thread moved to Trash')
    expect(ui$.toastUndo.peek()).toBeTruthy()

    runToastUndo()
    await nextTick()

    expect(calls.find((call) => call.command === 'mail.move')?.payload).toMatchObject({
      thread_id: 'acc#Trash#t.MQ',
      target_folder_id: 'inbox',
      message_ids: ['42'],
    })
    expect(mail$.threads.get().map((item) => item.thread_id)).toEqual(['acc#inbox#t.MQ'])
    expect(ui$.toast.get()).toBe('Thread moved')
    expect(ui$.toastUndo.peek()).toBeNull()
  })

  it('does not offer undo for permanent delete', async () => {
    responses['mail.delete'] = [{ ok: true, deleted: 1, permanent: true }]
    mail$.threads.set([
      thread({
        id: 'acc#Trash#t.MQ',
        folder_id: 'Trash',
        thread_id: 'acc#Trash#t.MQ',
      }),
    ])
    ui$.selectedThread.set('acc#Trash#t.MQ')
    ui$.selectedFolder.set('Trash')
    responses['mail.threadList'] = [{ threads: [] }]

    const pending = deleteThread('acc#Trash#t.MQ')
    settleConfirm(true)

    await pending

    expect(ui$.toast.get()).toBe('Thread deleted')
    expect(ui$.toastUndo.peek()).toBeNull()
  })
})

describe('archiveThread undo', () => {
  const calls: { command: string; payload: unknown }[] = []
  let responses: Record<string, unknown[]> = {}

  beforeEach(() => {
    calls.length = 0
    responses = {
      'mail.archive': [{ ok: true, moved: 1, folder: 'Archive', thread_id: 'acc#Archive#t.MQ', target_uids: [9] }],
      'mail.threadList': [{ threads: [] }, { threads: [thread({ thread_id: 'acc#inbox#t.MQ' })] }],
      'mail.move': [{ ok: true, moved: 1 }],
    }
    mail$.threads.set([thread({ thread_id: 'acc#inbox#t.MQ' })])
    mail$.messages.set([])
    mail$.folders.set([
      { id: 'inbox', account_id: 'acc', name: 'Inbox', role: 'inbox', unread: 0 },
      { id: 'Archive', account_id: 'acc', name: 'Archive', role: 'archive', unread: 0 },
    ])
    mail$.foldersByAccount.set({
      acc: [
        { id: 'inbox', account_id: 'acc', name: 'Inbox', role: 'inbox', unread: 0 },
        { id: 'Archive', account_id: 'acc', name: 'Archive', role: 'archive', unread: 0 },
      ],
    })
    kanban$.threads.set({})
    ui$.selectedThread.set('acc#inbox#t.MQ')
    ui$.selectedAccount.set('acc')
    ui$.selectedFolder.set('inbox')
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

  it('moves the archived copy back to the original folder', async () => {
    await archiveThread('acc#inbox#t.MQ')

    expect(mail$.threads.get()).toHaveLength(0)
    expect(ui$.toast.get()).toBe('Thread archived')
    expect(ui$.toastUndo.peek()).toBeTruthy()

    runToastUndo()
    await nextTick()

    expect(calls.find((call) => call.command === 'mail.move')?.payload).toMatchObject({
      thread_id: 'acc#Archive#t.MQ',
      target_folder_id: 'inbox',
      message_ids: ['9'],
    })
    expect(mail$.threads.get().map((item) => item.thread_id)).toEqual(['acc#inbox#t.MQ'])
    expect(ui$.toastTone.get()).toBe('success')
    expect(ui$.toast.get()).toBe('Thread moved')
  })

  it('rolls back and reports an error when archive moves no messages', async () => {
    responses['mail.archive'] = [{ ok: true, moved: 0, folder: 'Archive', thread_id: 'acc#Archive#t.MQ' }]
    responses['mail.threadList'] = []

    await archiveThread('acc#inbox#t.MQ')

    expect(calls.some((call) => call.command === 'mail.threadList')).toBe(false)
    expect(mail$.threads.get()).toHaveLength(1)
    expect(ui$.selectedThread.get()).toBe('acc#inbox#t.MQ')
    expect(ui$.toastTone.get()).toBe('error')
    expect(ui$.toast.get()).toBe('Archive failed: no matching messages found')
  })
})
