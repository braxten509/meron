// Supplies the bridge and editor state for desktop composer tests.
// Each suite can hold draft operations until a test releases them.
// Cleanup restores the editor and the fixture resets the bridge before each case.

import { afterEach, beforeEach } from 'bun:test'
import { cleanup } from '@testing-library/react'
import { accounts$ } from '../../states/accounts'
import { settings$ } from '../../states/settings'
import { compose$ } from '../../states/composeState'
import { kanban$ } from '../../states/kanban'
import { dismissToast, ui$ } from '../../states/ui'
import type { Account } from '../../types'

export const account = (id: string, signatureHtml: string): Account => ({
  id,
  email: `${id}@example.com`,
  display_name: id,
  provider: 'custom',
  auth_type: 'password',
  imap_host: 'imap.example.com',
  imap_port: 993,
  smtp_host: 'smtp.example.com',
  smtp_port: 465,
  tls: true,
  signature: { mode: 'custom', html: signatureHtml },
})

type Call = { command: string; payload: any }

export function composerFixture() {
  const bridge = {
    calls: [] as Call[],
    allocations: 0,
    holdAllocation: null as (() => void) | null,
    holdSave: null as (() => void) | null,
    holdAttachment: null as (() => void) | null,
    discardFailure: false,
    saveFailure: false,
    pendingDiscard: null as Promise<void> | null,
  }

  beforeEach(() => {
    bridge.calls = []
    bridge.allocations = 0
    bridge.holdAllocation = null
    bridge.holdSave = null
    bridge.holdAttachment = null
    bridge.discardFailure = false
    bridge.saveFailure = false
    bridge.pendingDiscard = null
    dismissToast()
    compose$.tabs.set([])
    compose$.activeTab.set('')
    // Other files can leave a conversation behind the tabs, which adds the
    // Current tab and a second "Close tab" button to the strip.
    compose$.conversationThread.set('')
    kanban$.activeBoardId.set('')
    kanban$.paneThreadId.set('')
    settings$.signature.set('')
    ui$.selectedAccount.set('a')
    accounts$.set([account('a', '<p>From A</p>'), account('b', '<p>From B</p>')])
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            bridge.calls.push({ command, payload })
            if (command === 'mail.allocateIdentity') {
              bridge.allocations += 1
              const id = `allocated-${bridge.allocations}@example.com`
              if (bridge.holdAllocation) {
                await new Promise<void>((resolve) => {
                  bridge.holdAllocation = resolve
                })
              }
              return { message_id: id }
            }
            if (command === 'mail.readAttachment') {
              if (bridge.holdAttachment) {
                await new Promise<void>((resolve) => {
                  bridge.holdAttachment = resolve
                })
              }
              return { data: 'AAAA', mime: 'application/pdf', size: 3 }
            }
            if (command === 'mail.saveDraft' && bridge.holdSave) {
              await new Promise<void>((resolve) => {
                bridge.holdSave = resolve
              })
            }
            if (command === 'mail.saveDraft' && bridge.saveFailure) throw new Error('server refused save')
            if (command === 'mail.discardDraft') {
              await bridge.pendingDiscard
              if (bridge.discardFailure) throw new Error('server refused discard')
            }
            return {}
          },
        },
      },
    }
  })

  afterEach(cleanup)

  const draftOf = (tabId: string) => compose$.tabs.peek().find((tab) => tab.id === tabId)?.compose
  const bodyHtml = () => document.querySelector('.tiptap-body')?.innerHTML ?? ''
  const savedDraftIds = () => bridge.calls.filter((c) => c.command === 'mail.saveDraft').map((c) => c.payload.draft_id)

  return { bridge, draftOf, bodyHtml, savedDraftIds }
}
