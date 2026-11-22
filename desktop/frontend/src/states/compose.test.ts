// Conversation and message tabs can open independently
// of the current mailbox. The fixtures exercise tab
// selection and preservation of the side navigation
// state.

import { beforeEach, describe, expect, it } from 'bun:test'
import type { MessageTab } from '../types'
import {
  activateConversationTab,
  closeMessageTab,
  closeMessageTabs,
  draftShouldOpenConversation,
  openDraftCompose,
  openDraftConversationOrCompose,
  openMessageTab,
  openComposeTab,
  openThreadTab,
  openThreadTabById,
} from './compose'
import { compose$ } from './composeState'
import { accounts$ } from './accounts'
import { ui$ } from './ui'
import { mail$ } from './mail'
import { message } from './composeTests/message'

describe('openThreadTabById', () => {
  const calls: { command: string; payload: unknown }[] = []

  beforeEach(() => {
    calls.length = 0
    compose$.tabs.set([])
    compose$.activeTab.set('')
    compose$.conversationThread.set('')
    mail$.messages.set([])
    mail$.messagesCursor.set('')
    mail$.messagesLoadingMore.set(false)
    mail$.threadLoading.set(false)
    accounts$.set([
      {
        id: 'acc-notification',
        email: 'me@example.com',
        display_name: 'Me',
        provider: 'custom',
        auth_type: 'password',
        imap_host: 'imap.example.com',
        imap_port: 993,
        smtp_host: 'smtp.example.com',
        smtp_port: 465,
        tls: true,
      },
    ])
    ui$.selectedAccount.set('acc-current')
    ui$.selectedFolder.set('work')
    ui$.selectedThread.set('t-current')
    ui$.query.set('needle')
    ui$.mobilePane.set('threads')
    ui$.toast.set('')
    ui$.toastTone.set('success')
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            if (command === 'mail.threadRead') {
              return { messages: [message()] }
            }
            return {}
          },
        },
      },
    }
  })

  it('opens a fetched thread tab without changing side navigation selection state', async () => {
    await openThreadTabById('t-notification')

    expect(compose$.activeTab.get()).toBe('thread-t-notification')
    expect(compose$.tabs.get()).toHaveLength(1)
    expect(compose$.tabs.get()[0]).toMatchObject({
      kind: 'thread',
      threadId: 't-notification',
      accountId: 'acc-notification',
      folderId: 'inbox',
      subject: 'Notification thread',
    })
    expect(ui$.selectedThread.get()).toBe('t-notification')
    expect(ui$.mobilePane.get()).toBe('conversation')
    expect(ui$.selectedAccount.get()).toBe('acc-current')
    expect(ui$.selectedFolder.get()).toBe('work')
    expect(ui$.query.get()).toBe('needle')
    expect(calls.filter((call) => call.command === 'mail.threadRead')).toHaveLength(1)
  })

  it('activates an existing thread tab without duplicating or fetching', async () => {
    openThreadTab(message())
    ui$.selectedAccount.set('acc-current')
    ui$.selectedFolder.set('work')
    ui$.selectedThread.set('t-current')
    calls.length = 0

    await openThreadTabById('t-notification')

    expect(compose$.activeTab.get()).toBe('thread-t-notification')
    expect(compose$.tabs.get()).toHaveLength(1)
    expect(ui$.selectedThread.get()).toBe('t-notification')
    expect(ui$.selectedAccount.get()).toBe('acc-current')
    expect(ui$.selectedFolder.get()).toBe('work')
    expect(calls.some((call) => call.command === 'mail.threadRead')).toBe(false)
  })

  it('preserves navigation state and shows an error when the thread cannot load', async () => {
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.threadRead') throw new Error('backend unavailable')
      return {}
    }

    await openThreadTabById('t-missing')

    expect(compose$.tabs.get()).toHaveLength(0)
    expect(compose$.activeTab.get()).toBe('')
    expect(ui$.selectedThread.get()).toBe('t-current')
    expect(ui$.selectedAccount.get()).toBe('acc-current')
    expect(ui$.selectedFolder.get()).toBe('work')
    expect(ui$.toastTone.get()).toBe('error')
    expect(ui$.toast.get()).toBe('backend unavailable')
  })

  it('opens a Drafts row as an editable compose tab', async () => {
    const draft = message({
      id: 'acc-notification#Drafts#42#99',
      folder_id: 'Drafts',
      thread_id: 'acc-notification#Drafts#42',
      from_addr: 'me@example.com',
      to: 'you@example.com',
      cc: 'copy@example.com',
      bcc: 'blind@example.com',
      message_id: 'draft-id@example.com',
      subject: 'Draft subject',
      body: 'saved body',
    })
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.threadRead') return { messages: [draft] }
      return {}
    }

    const handled = await openDraftCompose(draft)

    expect(handled).toBe(true)
    expect(compose$.tabs.get()).toHaveLength(1)
    expect(compose$.activeTab.get()).toBe(compose$.tabs.get()[0].id)
    expect(compose$.tabs.get()[0]).toMatchObject({
      kind: 'compose',
      subject: 'Draft subject',
      threadId: '',
      compose: {
        accountId: 'acc-notification',
        fromEmail: 'me@example.com',
        to: 'you@example.com',
        cc: 'copy@example.com',
        bcc: 'blind@example.com',
        text: 'saved body',
        draftMessageId: 'draft-id@example.com',
        sourceDraft: {
          threadId: 'acc-notification#Drafts#42',
          messageId: 'acc-notification#Drafts#42#99',
          folderId: 'Drafts',
        },
      },
    })
  })

  it('reuses the open compose tab for repeated Drafts row clicks', async () => {
    const draft = message({
      id: 'acc-notification#Drafts#42#99',
      folder_id: 'Drafts',
      thread_id: 'acc-notification#Drafts#42',
      from_addr: 'me@example.com',
      to: 'you@example.com',
      message_id: 'draft-id@example.com',
      subject: 'Draft subject',
      body: 'saved body',
    })
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.threadRead') return { messages: [draft] }
      return {}
    }

    await openDraftCompose(draft)
    const firstTabId = compose$.activeTab.get()
    await openDraftCompose({
      ...draft,
      id: 'acc-notification#DRAFTS#42#100',
      folder_id: 'DRAFTS',
      thread_id: 'acc-notification#DRAFTS#42',
      message_id: '<DRAFT-ID@example.com>',
    })

    expect(compose$.tabs.get()).toHaveLength(1)
    expect(compose$.activeTab.get()).toBe(firstTabId)
    expect(calls.filter((call) => call.command === 'mail.threadRead')).toHaveLength(1)
  })

  it('opens a Drafts row with a cached ancestor in the conversation view', async () => {
    const ancestor = message({
      id: 'acc-notification#INBOX#42#1',
      folder_id: 'INBOX',
      thread_id: 'acc-notification#Drafts#42',
      from_addr: 'you@example.com',
      message_id: 'root@example.com',
      subject: 'Thread subject',
    })
    const draft = message({
      id: 'acc-notification#Drafts#42#99',
      folder_id: 'Drafts',
      thread_id: 'acc-notification#Drafts#42',
      from_addr: 'me@example.com',
      to: 'you@example.com',
      subject: 'Re: Thread subject',
      body: 'draft reply',
    })
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.threadRead') return { messages: [ancestor, draft] }
      return {}
    }

    const handled = await openDraftConversationOrCompose(draft)

    expect(handled).toBe(true)
    expect(compose$.tabs.get()).toHaveLength(0)
    expect(compose$.activeTab.get()).toBe('')
    expect(ui$.selectedThread.get()).toBe('acc-notification#Drafts#42')
    expect(ui$.mobilePane.get()).toBe('conversation')
    expect(mail$.messages.get()).toEqual([ancestor, draft])
  })

  it('opens a referenced Drafts row in the conversation view even without cached ancestors', async () => {
    const draft = message({
      id: 'acc-notification#Drafts#42#99',
      folder_id: 'Drafts',
      thread_id: 'acc-notification#Drafts#42',
      from_addr: 'me@example.com',
      to: 'you@example.com',
      subject: 'Re: Thread subject',
      references: 'root@example.com',
    })
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.threadRead') return { messages: [draft] }
      return {}
    }

    await openDraftConversationOrCompose(draft)

    expect(compose$.tabs.get()).toHaveLength(0)
    expect(ui$.selectedThread.get()).toBe('acc-notification#Drafts#42')
    expect(mail$.messages.get()).toEqual([draft])
  })

  it('classifies standalone Drafts rows as composer-only', () => {
    const draft = message({ folder_id: 'Drafts', references: '' })

    expect(draftShouldOpenConversation([draft], draft)).toBe(false)
  })

  it('opens a standalone Drafts row through the shared helper as an editable compose tab', async () => {
    const draft = message({
      id: 'acc-notification#Drafts#42#99',
      folder_id: 'Drafts',
      thread_id: 'acc-notification#Drafts#42',
      from_addr: 'me@example.com',
      to: 'you@example.com',
      message_id: 'draft-id@example.com',
      subject: 'Draft subject',
      body: 'saved body',
    })
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.threadRead') return { messages: [draft] }
      return {}
    }

    await openDraftConversationOrCompose(draft)

    expect(compose$.tabs.get()).toHaveLength(1)
    expect(compose$.activeTab.get()).toBe(compose$.tabs.get()[0].id)
    expect(compose$.tabs.get()[0]).toMatchObject({
      kind: 'compose',
      subject: 'Draft subject',
      compose: {
        to: 'you@example.com',
        text: 'saved body',
        draftMessageId: 'draft-id@example.com',
      },
    })
  })
})

describe('tab navigation', () => {
  beforeEach(() => {
    compose$.tabs.set([])
    compose$.activeTab.set('')
    compose$.conversationThread.set('')
    ui$.selectedThread.set('')
    accounts$.set([
      {
        id: 'acc-notification',
        email: 'me@example.com',
        display_name: 'Me',
        provider: 'custom',
        auth_type: 'password',
        imap_host: 'imap.example.com',
        imap_port: 993,
        smtp_host: 'smtp.example.com',
        smtp_port: 465,
        tls: true,
      },
    ])
  })

  it('remembers the Current conversation while a thread tab is active', () => {
    // Selecting a thread in the list mirrors into conversationThread.
    ui$.selectedThread.set('t-current')
    expect(compose$.conversationThread.get()).toBe('t-current')

    // Opening a thread tab retargets selectedThread to load its own messages,
    // but must not disturb the Current tab's remembered thread.
    openThreadTab(message({ thread_id: 't-2', id: 'm2', subject: 'Thread 2' }))
    expect(compose$.activeTab.get()).toBe('thread-t-2')
    expect(ui$.selectedThread.get()).toBe('t-2')
    expect(compose$.conversationThread.get()).toBe('t-current')
  })

  it('restores the Current conversation when switching back to the Current tab', () => {
    ui$.selectedThread.set('t-current')
    openThreadTab(message({ thread_id: 't-2', id: 'm2' }))
    expect(ui$.selectedThread.get()).toBe('t-2')

    activateConversationTab()
    expect(compose$.activeTab.get()).toBe('')
    expect(ui$.selectedThread.get()).toBe('t-current')
  })

  it('returns to the originating thread tab when a reader tab opened from it is closed', () => {
    ui$.selectedThread.set('t-current')
    openThreadTab(message({ thread_id: 't-2', id: 'm2', subject: 'Thread 2' }))

    // Open a message from within the thread tab in its own reader tab.
    openMessageTab(message({ id: 'msg-x', thread_id: 't-2', subject: 'Msg X' }))
    expect(compose$.activeTab.get()).toBe('msg-x')

    // Closing it returns to the thread tab it was opened from, not a neighbor.
    void closeMessageTab('msg-x')
    expect(compose$.activeTab.get()).toBe('thread-t-2')
    expect(ui$.selectedThread.get()).toBe('t-2')
    expect(compose$.tabs.get().map((tab) => tab.id)).toEqual(['thread-t-2'])
  })

  it("refreshes the reader tab's message once a missing body arrives", () => {
    openMessageTab(message({ id: 'msg-late', body: '', body_missing: true }))
    expect(compose$.tabs.get()[0].message?.body_missing).toBe(true)

    const attachment = { filename: 'a.pdf', mime: 'application/pdf', size: 3, key: 'k', url: null }
    mail$.messages.set([message({ id: 'msg-late', body: 'Full body', attachments: [attachment] })])

    // The bottom bar forwards from the snapshot, so it has to carry what the
    // reader now shows, not what was there when the tab opened.
    const tab = compose$.tabs.get()[0]
    expect(tab.bodyMissing).toBe(false)
    expect(tab.message?.body).toBe('Full body')
    expect(tab.message?.body_missing).toBeFalsy()
    expect(tab.message?.attachments).toEqual([attachment])
  })

  it('snapshots message attachments when opening a reader tab', () => {
    openMessageTab(
      message({
        id: 'msg-image',
        attachments: [
          {
            filename: 'photo.png',
            mime: 'image/png',
            size: 42,
            key: null,
            url: 'data:image/png;base64,aW1hZ2U=',
          },
        ],
      }),
    )

    expect(compose$.tabs.get()[0].attachments).toEqual([
      {
        filename: 'photo.png',
        mime: 'image/png',
        size: 42,
        key: null,
        url: 'data:image/png;base64,aW1hZ2U=',
      },
    ])
  })

  it('hands over to a remaining tab when there is no Current conversation', () => {
    // Tabs restored from the previous run: nothing in the activation trail, and
    // no conversation behind them (the pane was empty). The Current tab has
    // nothing to show — and in kanban view it closes the pane — so closing the
    // active tab moves to a tab that's still open.
    const restored = (threadId: string): MessageTab => ({
      id: `thread-${threadId}`,
      kind: 'thread',
      messageId: '',
      threadId,
      subject: threadId,
      from: 'a@example.com',
      body: '',
      viewMode: 'plain',
    })
    compose$.tabs.set([restored('t-a'), restored('t-b')])
    compose$.activeTab.set('thread-t-b')

    void closeMessageTab('thread-t-b')
    expect(compose$.activeTab.get()).toBe('thread-t-a')
    expect(ui$.selectedThread.get()).toBe('t-a')
  })

  it('falls back to the Current conversation when the last tab is closed', () => {
    ui$.selectedThread.set('t-current')
    openThreadTab(message({ thread_id: 't-2', id: 'm2' }))

    void closeMessageTab('thread-t-2')
    expect(compose$.activeTab.get()).toBe('')
    expect(ui$.selectedThread.get()).toBe('t-current')
    expect(compose$.tabs.get()).toHaveLength(0)
  })

  it('bulk-closes thread and reader tabs but keeps compose tabs and their drafts', () => {
    ui$.selectedThread.set('t-current')
    openThreadTab(message({ thread_id: 't-2', id: 'm2' }))
    const composeId = openComposeTab({ subject: 'Unsent' })!
    openMessageTab(message({ id: 'msg-x', thread_id: 't-3' }))
    openThreadTab(message({ thread_id: 't-4', id: 'm4' }))

    closeMessageTabs(compose$.tabs.get().map((tab) => tab.id))
    expect(compose$.tabs.get().map((tab) => tab.id)).toEqual([composeId])
    // The active tab closes last, so it lands on the compose tab still open
    // rather than on a tab that was about to go.
    expect(compose$.activeTab.get()).toBe(composeId)
  })
})
