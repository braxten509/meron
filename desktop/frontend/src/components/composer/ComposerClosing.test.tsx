// The desktop composer manages an editor alongside asynchronous
// draft and send commands. The fixtures hold selected bridge
// responses so tests can control ownership, closure and delivery
// order.

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { act, fireEvent, render } from '@testing-library/react'
import { draftOpenInComposeTab } from '../../states/quickReply'
import {
  closeMessageTab,
  forwardMessage,
  openComposeTab,
  openDraftCompose,
  updateComposeDraft,
} from '../../states/compose'
import { compose$ } from '../../states/composeState'
import { kanban$ } from '../../states/kanban'
import { runToastUndo, ui$ } from '../../states/ui'
import type { ComposeDraft, Message } from '../../types'
import { Composer } from './Composer'
import { ConversationTabs } from '../chat/ConversationTabs'
import { AppHotkeys } from '../dialog/AppHotkeys'

import { composerFixture } from './composerFixture'

describe('Closing a composer', () => {
  const { bridge, draftOf, bodyHtml, savedDraftIds } = composerFixture()

  it('deletes the server draft when the composer is discarded mid-save', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const view = render(<Composer tabId={tabId} />)

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })

    await act(async () => {
      const discard = [...view.container.querySelectorAll('button')].find((b) => b.textContent === 'Discard')!
      discard.click()
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    const order = bridge.calls.map((call) => call.command)
    const discarded = order.lastIndexOf('mail.discardDraft')
    expect(discarded).toBeGreaterThan(-1)
    // The draft deleted is the one the save created, and nothing re-saves it.
    expect(bridge.calls[discarded].payload.draft_id).toBe(savedDraftIds()[0])
    expect(order.slice(discarded).includes('mail.saveDraft')).toBe(false)
  })

  // The tab strip's × keeps the message in Drafts, like Esc; throwing it away
  // is the Discard button's job.
  it('drains and saves to Drafts when the tab strip closes the composer', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const view = render(
      <>
        <ConversationTabs />
        <Composer tabId={tabId} />
      </>,
    )

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      view.getByTitle('Close tab').click()
      expect(draftOf(tabId)).toBeUndefined()
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    // The last save ran after the one in flight (which, finding the tab gone,
    // cleans up its own copy), and exactly that last copy is left in Drafts.
    const order = bridge.calls.map((call) => call.command)
    expect(order.lastIndexOf('mail.saveDraft')).toBeGreaterThan(order.lastIndexOf('mail.discardDraft'))
    const discarded = new Set(
      bridge.calls.filter((call) => call.command === 'mail.discardDraft').map((call) => call.payload.draft_id),
    )
    const ids = savedDraftIds()
    const kept = [...new Set(ids)].filter((id) => !discarded.has(id))
    expect(kept).toEqual([ids[ids.length - 1]])
    expect(draftOf(tabId)).toBeUndefined()
  })

  // A save that fails may still have written a copy. The tab comes back (so
  // nothing typed is lost) under the id of the copy it may have left, and any
  // other copy written meanwhile is discarded: no stray drafts pile up.
  it('discards the copy a failed save may have written after the tab closed', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const view = render(
      <>
        <ConversationTabs />
        <Composer tabId={tabId} />
      </>,
    )

    bridge.holdAllocation = () => {}
    bridge.saveFailure = true
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      view.getByTitle('Close tab').click()
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    const reopened = draftOf(tabId)
    expect(reopened).toBeDefined()
    const discarded = new Set(
      bridge.calls.filter((call) => call.command === 'mail.discardDraft').map((call) => call.payload.draft_id),
    )
    const stray = [...new Set(savedDraftIds())].filter((id) => id !== reopened?.draftMessageId && !discarded.has(id))
    expect(stray).toEqual([])
  })

  it('closes on Esc at once and keeps the message in Drafts', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    compose$.activeTab.set(tabId)
    const view = render(<Composer tabId={tabId} />)

    // A slow Drafts server: the tab must not wait for it.
    bridge.holdAllocation = () => {}
    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Hello'), { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    expect(draftOf(tabId)).toBeUndefined()

    await act(async () => {
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })
    // Saved without waiting for the autosave debounce, and never discarded.
    expect(savedDraftIds()).toEqual(['allocated-1@example.com'])
    expect(bridge.calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
  })

  it('closes on Esc from the rich-text body, which marks the key handled', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', html: '<p>hi</p>', rich: true })!
    compose$.activeTab.set(tabId)
    render(<Composer tabId={tabId} />)

    // ProseMirror calls preventDefault on Esc (keyCode 27) in the editor.
    await act(async () => {
      fireEvent.keyDown(document.querySelector('.tiptap-body')!, { key: 'Escape', keyCode: 27 })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(draftOf(tabId)).toBeUndefined()
    expect(savedDraftIds()).toEqual(['allocated-1@example.com'])
  })

  it('keeps the draft when Esc comes while an autosave allocates it', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    const view = render(<Composer tabId={tabId} />)

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Hello'), { key: 'Escape' })
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    // The autosave cleans up the copy it allocated for the closed tab; the
    // save on Esc, which runs after it, writes the one that stays.
    const order = bridge.calls.map((call) => call.command)
    expect(order.lastIndexOf('mail.saveDraft')).toBeGreaterThan(order.lastIndexOf('mail.discardDraft'))
    expect(savedDraftIds().at(-1)).toBe('allocated-2@example.com')
  })

  it('brings the tab back when the save on Esc fails', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    compose$.activeTab.set(tabId)
    const view = render(<Composer tabId={tabId} />)

    bridge.saveFailure = true
    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Hello'), { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(draftOf(tabId)?.text).toStartWith('hi')
    expect(compose$.activeTab.peek()).toBe(tabId)
    expect(bridge.calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
  })

  it('keeps the edits of a failed save when the draft was reopened meanwhile', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'latest', rich: false })!
    const view = render(<Composer tabId={tabId} />)

    bridge.saveFailure = true
    bridge.holdAllocation = () => {}
    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Hello'), { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    // The older copy, opened from Drafts while the save is still running.
    const older = openComposeTab({ draftMessageId: 'allocated-1@example.com', text: 'older', rich: false })!
    await act(async () => {
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(draftOf(tabId)?.text).toStartWith('latest')
    expect(draftOf(tabId)?.draftMessageId).toStartWith('local-draft-')
    expect(draftOf(older)?.draftMessageId).toBe('allocated-1@example.com')
    expect(compose$.activeTab.peek()).toBe(tabId)
  })

  const draftRow = (messageId = 'saved@example.com') =>
    ({
      id: `a#Drafts#${messageId}`,
      account_id: 'a',
      thread_id: `a#Drafts#t-${messageId}`,
      folder_id: 'Drafts',
      message_id: messageId,
      subject: 'Hello',
      body: 'older',
    }) as unknown as Message

  const openSavedDraftTab = () => {
    const tabId = openComposeTab({
      to: 'x@example.com',
      subject: 'Hello',
      text: 'latest',
      rich: false,
      draftMessageId: 'saved@example.com',
    })!
    return { tabId, view: render(<Composer tabId={tabId} />) }
  }

  const pressEscWithSaveHeld = async (input: HTMLElement) => {
    bridge.holdSave = () => {}
    await act(async () => {
      fireEvent.keyDown(input, { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
  }

  const releaseSave = () => {
    bridge.holdSave?.()
    bridge.holdSave = null
  }

  const escThenOpenRow = async (input: HTMLElement) => {
    await pressEscWithSaveHeld(input)
    const opening = openDraftCompose(draftRow())
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    // Still waiting for the save: nothing opened from the older row.
    expect(compose$.tabs.peek()).toHaveLength(0)
    await act(async () => {
      releaseSave()
      await opening
    })
  }

  it('opens a draft from its Esc save, not the older row it replaces', async () => {
    const { tabId, view } = openSavedDraftTab()

    await escThenOpenRow(view.getByDisplayValue('Hello'))

    expect(compose$.tabs.peek().map((tab) => tab.id)).toEqual([tabId])
    expect(draftOf(tabId)?.text).toStartWith('latest')
    expect(compose$.activeTab.peek()).toBe(tabId)
    expect(bridge.calls.some((call) => call.command === 'mail.threadRead')).toBe(false)
  })

  it('opens the restored tab when the Esc save fails while a draft row opens', async () => {
    const { tabId, view } = openSavedDraftTab()
    bridge.saveFailure = true

    await escThenOpenRow(view.getByDisplayValue('Hello'))

    expect(compose$.tabs.peek().map((tab) => tab.id)).toEqual([tabId])
    expect(draftOf(tabId)?.text).toStartWith('latest')
    expect(compose$.activeTab.peek()).toBe(tabId)
  })

  it('counts a draft as open in a tab until its Esc save lands', async () => {
    const { view } = openSavedDraftTab()
    await pressEscWithSaveHeld(view.getByDisplayValue('Hello'))

    expect(draftOpenInComposeTab('saved@example.com')).toBe(true)

    await act(async () => {
      releaseSave()
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    expect(draftOpenInComposeTab('saved@example.com')).toBe(false)
  })

  it('opens another draft without waiting for an Esc save', async () => {
    const { view } = openSavedDraftTab()
    await pressEscWithSaveHeld(view.getByDisplayValue('Hello'))

    await act(async () => {
      await openDraftCompose(draftRow('other@example.com'))
    })

    expect(bridge.calls.some((call) => call.command === 'mail.threadRead')).toBe(true)
    expect(compose$.tabs.peek().map((tab) => tab.compose?.draftMessageId)).toEqual(['other@example.com'])

    await act(async () => {
      releaseSave()
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
  })

  const forwardWithAttachmentHeld = async () => {
    bridge.holdAttachment = () => {}
    void forwardMessage({
      id: 'a#INBOX#1#1',
      account_id: 'a',
      subject: 'Report',
      body: 'see attached',
      attachments: [{ key: 'k1', filename: 'report.pdf', mime: 'application/pdf', size: 3 }],
    } as unknown as Message)
    const tabId = compose$.activeTab.peek()
    const view = render(<Composer tabId={tabId} />)
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    return { tabId, view }
  }

  const savedAttachmentNames = () =>
    bridge.calls
      .filter((c) => c.command === 'mail.saveDraft')
      .map((c) => c.payload.attachments.map((a: any) => a.filename))

  it('waits for attachments still on their way before the save on Esc', async () => {
    const { tabId, view } = await forwardWithAttachmentHeld()

    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Fwd: Report'), { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    // Closed at once, with nothing saved without the attachment.
    expect(draftOf(tabId)).toBeUndefined()
    expect(savedAttachmentNames()).toEqual([])

    await act(async () => {
      bridge.holdAttachment?.()
      await new Promise((resolve) => setTimeout(resolve, 50))
    })
    expect(savedAttachmentNames()).toEqual([['report.pdf']])

    runToastUndo()
    expect(draftOf(tabId)?.attachments.map((a) => a.filename)).toEqual(['report.pdf'])
  })

  it('brings attachments back with the tab when the save on Esc fails', async () => {
    const { tabId, view } = await forwardWithAttachmentHeld()
    bridge.saveFailure = true

    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Fwd: Report'), { key: 'Escape' })
      bridge.holdAttachment?.()
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(draftOf(tabId)?.attachments.map((a) => a.filename)).toEqual(['report.pdf'])
  })

  it('closes the compose tab in front on Esc from outside it, not the pane behind', async () => {
    kanban$.activeBoardId.set('board')
    kanban$.paneThreadId.set('a#INBOX#1')
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    compose$.activeTab.set(tabId)
    render(
      <>
        <AppHotkeys />
        <Composer tabId={tabId} />
      </>,
    )

    await act(async () => {
      fireEvent.keyDown(document.body, { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(savedDraftIds()).toEqual(['allocated-1@example.com'])
    expect(draftOf(tabId)).toBeUndefined()
    expect(kanban$.paneThreadId.peek()).toBe('a#INBOX#1')
  })

  it('leaves Esc to an IME composing text', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    const view = render(<Composer tabId={tabId} />)

    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Hello'), { key: 'Escape', isComposing: true })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(draftOf(tabId)).toBeDefined()
  })

  it('leaves to an IME the Esc WebKit reports as keyCode 229', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    const view = render(<Composer tabId={tabId} />)

    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Hello'), { key: 'Escape', keyCode: 229 })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(draftOf(tabId)).toBeDefined()
  })

  it('keeps what was typed before the tab was switched away on Esc', async () => {
    const tabId = openComposeTab({ rich: false })!
    const first = render(<Composer tabId={tabId} />)
    await act(async () => {
      updateComposeDraft(tabId, { text: 'written, then the tab was left' })
    })
    // An inactive tab is unmounted, so coming back mounts a new composer.
    first.unmount()
    const view = render(<Composer tabId={tabId} />)

    await act(async () => {
      fireEvent.keyDown(view.container.querySelector('textarea')!, { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(savedDraftIds()).toEqual(['allocated-1@example.com'])
    expect(bridge.calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
  })

  it('keeps a body seeded with no recipient or subject on Esc', async () => {
    const tabId = openComposeTab({ text: 'from a mailto body' })!
    const view = render(<Composer tabId={tabId} />)

    await act(async () => {
      fireEvent.keyDown(view.container.querySelector('textarea')!, { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(savedDraftIds()).toEqual(['allocated-1@example.com'])
  })

  it('says the message went to Drafts and reopens it from the toast', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    compose$.activeTab.set(tabId)
    const view = render(<Composer tabId={tabId} />)

    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Hello'), { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })
    expect(ui$.toast.peek()).toBe('Saved to Drafts')
    expect(ui$.toastActionLabel.peek()).toBe('Open')

    runToastUndo()

    // Under the saved id, so the next save replaces that copy.
    expect(draftOf(tabId)?.draftMessageId).toBe('allocated-1@example.com')
    expect(draftOf(tabId)?.text).toStartWith('hi')
    expect(compose$.activeTab.peek()).toBe(tabId)
  })

  it('goes to the tab already holding the draft when opened from the toast', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    const view = render(<Composer tabId={tabId} />)

    await act(async () => {
      fireEvent.keyDown(view.getByDisplayValue('Hello'), { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })
    const fromDrafts = openComposeTab({ draftMessageId: 'allocated-1@example.com', text: 'hi', rich: false })!
    compose$.activeTab.set('')

    runToastUndo()

    expect(compose$.tabs.peek().map((tab) => tab.id)).toEqual([fromDrafts])
    expect(compose$.activeTab.peek()).toBe(fromDrafts)
  })

  const pressEscIn = async (tabId: string, body: Partial<ComposeDraft>) => {
    await act(async () => {
      updateComposeDraft(tabId, body)
    })
    await act(async () => {
      fireEvent.keyDown(document.querySelector('.tiptap-body, textarea')!, { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })
  }

  it('throws away a body typed in and emptied again on Esc', async () => {
    const tabId = openComposeTab({ rich: true })!
    render(<Composer tabId={tabId} />)

    // What the editor reports once everything typed above the signature is gone.
    await pressEscIn(tabId, { html: '<p></p><p></p><p>From A</p>' })

    expect(draftOf(tabId)).toBeUndefined()
    expect(savedDraftIds()).toEqual([])
  })

  it('throws away a body of only white space on Esc', async () => {
    const tabId = openComposeTab({ rich: false })!
    render(<Composer tabId={tabId} />)

    await pressEscIn(tabId, { text: '  \n \n\nFrom A' })

    expect(savedDraftIds()).toEqual([])
    expect(ui$.toast.peek()).toBe('')
  })

  it('keeps an image with no text around it on Esc', async () => {
    const tabId = openComposeTab({ rich: true })!
    render(<Composer tabId={tabId} />)

    await pressEscIn(tabId, { html: '<p><img src="https://example.com/a.png"></p><p></p><p>From A</p>' })

    expect(savedDraftIds()).toEqual(['allocated-1@example.com'])
  })

  it('keeps a signature that was edited on Esc', async () => {
    const tabId = openComposeTab({ rich: true })!
    render(<Composer tabId={tabId} />)

    await pressEscIn(tabId, { html: '<p></p><p>From A, and more</p>' })

    expect(savedDraftIds()).toEqual(['allocated-1@example.com'])
  })

  it('throws away a new message nothing was written in on Esc', async () => {
    const tabId = openComposeTab({ rich: false })!
    const view = render(<Composer tabId={tabId} />)

    await act(async () => {
      fireEvent.keyDown(view.container.querySelector('textarea')!, { key: 'Escape' })
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(savedDraftIds()).toEqual([])
    expect(draftOf(tabId)).toBeUndefined()
  })

  it('deduplicates simultaneous footer and external close requests', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const view = render(
      <>
        <ConversationTabs />
        <Composer tabId={tabId} />
      </>,
    )

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      const discard = [...view.container.querySelectorAll('button')].find((button) => button.textContent === 'Discard')!
      discard.click()
      const externalClose = closeMessageTab(tabId)
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await externalClose
      await new Promise((resolve) => setTimeout(resolve, 20))
    })

    expect(bridge.calls.filter((call) => call.command === 'mail.discardDraft')).toHaveLength(1)
    expect(draftOf(tabId)).toBeUndefined()
  })

  for (const mounted of [true, false]) {
    it(`closes an empty ${mounted ? 'mounted' : 'unmounted'} composer before remote discard finishes`, async () => {
      const tabId = openComposeTab()!
      updateComposeDraft(tabId, { draftMessageId: 'saved-empty@example.com' })
      if (mounted) render(<Composer tabId={tabId} />)

      let releaseDiscard!: () => void
      bridge.pendingDiscard = new Promise<void>((resolve) => {
        releaseDiscard = resolve
      })
      let closing!: Promise<void>
      try {
        await act(async () => {
          closing = closeMessageTab(tabId)
        })
        expect(draftOf(tabId)).toBeUndefined()
        expect(bridge.calls.filter((call) => call.command === 'mail.discardDraft')).toHaveLength(1)
        await closeMessageTab(tabId)
        expect(bridge.calls.filter((call) => call.command === 'mail.discardDraft')).toHaveLength(1)
      } finally {
        await act(async () => {
          releaseDiscard()
          await closing
        })
      }
    })
  }

  describe('when a discard fails', () => {
    const realSetTimeout = globalThis.setTimeout
    const wait = (ms: number) =>
      act(async () => {
        await new Promise((resolve) => realSetTimeout(resolve, ms))
      })
    const discardWithFailure = async () => {
      const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
      const view = render(<Composer tabId={tabId} />)
      await act(async () => {
        updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
      })
      await wait(20)
      bridge.discardFailure = true
      await act(async () => {
        ;[...view.container.querySelectorAll('button')].find((button) => button.textContent === 'Discard')!.click()
      })
      await wait(10)
      return tabId
    }

    beforeEach(() => {
      // The retry's delay, cut short.
      globalThis.setTimeout = ((handler: () => void, ms?: number) =>
        realSetTimeout(handler, ms === 5_000 ? 40 : ms)) as typeof setTimeout
    })
    afterEach(() => {
      globalThis.setTimeout = realSetTimeout
    })

    it('closes the composer and reports it once the retry fails too', async () => {
      const tabId = await discardWithFailure()
      expect(draftOf(tabId)).toBeUndefined()
      // The first failure may only be a slow connection.
      expect(ui$.toast.peek()).toBe('')

      await wait(80)
      expect(ui$.toast.peek()).toBe('Could not discard draft: server refused discard')
    })

    it('says nothing when the retry succeeds', async () => {
      await discardWithFailure()
      bridge.discardFailure = false

      await wait(80)
      expect(ui$.toast.peek()).toBe('')
    })
  })
})
