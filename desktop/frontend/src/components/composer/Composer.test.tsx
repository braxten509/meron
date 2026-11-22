// The desktop composer manages an editor alongside asynchronous
// draft and send commands. The fixtures hold selected bridge
// responses so tests can control ownership, closure and delivery
// order.

import { describe, expect, it } from 'bun:test'
import { act, cleanup, fireEvent, render } from '@testing-library/react'
import { accounts$ } from '../../states/accounts'
import { openComposeTab, updateComposeDraft } from '../../states/compose'
import { settleConfirm } from '../../states/ui'
import { Composer } from './Composer'
import { AppConfirm } from '../dialog/AppConfirm'

import { account, composerFixture } from './composerFixture'

describe('Composer', () => {
  const { bridge, draftOf, bodyHtml, savedDraftIds } = composerFixture()

  it('scales the plain-text body with the message text size', () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    const view = render(<Composer tabId={tabId} />)

    const body = view.container.querySelector('textarea')
    expect(body?.className).toContain('var(--me-message-scale)')
  })

  it('assigns scrolling and padding to the shrinkable plain-text body', () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi', rich: false })!
    const view = render(<Composer tabId={tabId} />)

    // These assertions check the layout contract, not browser geometry.
    // The textarea owns scrolling; its flex wrapper must not scroll too.
    const body = view.container.querySelector('textarea')!
    const classes = body.className.split(' ')
    expect(classes).toContain('block')
    expect(classes).toContain('px-4')
    expect(classes).toContain('py-3')
    expect(classes).toContain('min-h-0')
    expect(classes).toContain('flex-1')
    expect(classes).not.toContain('min-h-[240px]')
    expect(classes).not.toContain('h-full')
    const wrapperClasses = body.parentElement!.className.split(' ')
    expect(wrapperClasses).toContain('flex')
    expect(wrapperClasses).toContain('flex-col')
    expect(wrapperClasses).toContain('min-h-0')
    expect(wrapperClasses).toContain('overflow-hidden')
    expect(wrapperClasses).not.toContain('overflow-y-auto')
    expect(body.parentElement!.className.split(' ')).not.toContain('px-4')
    expect(body.parentElement!.className.split(' ')).not.toContain('py-3')
  })

  it('keeps the side padding on the scrolling body above a folded quote', () => {
    const quote = { rich: false, html: '<p>On Mon, Them wrote:</p>', text: 'On Mon, Them wrote:\n> Lunch?' }
    const tabId = openComposeTab({ to: 'them@example.com', rich: false, text: 'Sounds good', quote })!
    const view = render(<Composer tabId={tabId} />)

    const body = view.container.querySelector('textarea')!
    expect(body.className.split(' ')).toContain('block')
    expect(body.className.split(' ')).not.toContain('px-4')
    expect(body.className.split(' ')).not.toContain('py-3')
    expect(body.parentElement!.className.split(' ')).toContain('px-4')
    expect(body.parentElement!.className.split(' ')).toContain('py-3')
    expect(body.parentElement!.className.split(' ')).toContain('overflow-y-auto')
  })

  it('scales the rich-text body with the message text size', () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', rich: true })!
    render(<Composer tabId={tabId} />)

    expect(document.querySelector('.tiptap-body')?.className).toContain('var(--me-message-scale)')
  })

  it('allocates one server draft when autosaves overlap', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    render(<Composer tabId={tabId} />)

    // Hold the first allocation open, then start a second save behind it: this
    // is the window in which both used to allocate a draft of their own.
    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'alias@example.com' })
    })
    await act(async () => {
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(bridge.allocations).toBe(1)
    const ids = savedDraftIds()
    expect(ids.length).toBeGreaterThan(1)
    // Every save wrote to the same server draft, so none was orphaned.
    expect(new Set(ids).size).toBe(1)
    expect(draftOf(tabId)?.draftMessageId).toBe(ids[0])
  })

  it('keeps one autosave queue when the composer unmounts and remounts during allocation', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const first = render(<Composer tabId={tabId} />)

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    first.unmount()
    render(<Composer tabId={tabId} />)
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'alias@example.com' })
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    const ids = savedDraftIds()
    expect(bridge.allocations).toBe(1)
    expect(ids.length).toBeGreaterThan(1)
    expect(new Set(ids).size).toBe(1)
    expect(draftOf(tabId)?.draftMessageId).toBe(ids[0])
  })

  it('never writes the draft back to the server after it has been sent', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const view = render(<Composer tabId={tabId} />)

    // A save is in flight (and another queued behind it) when Send is pressed.
    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'alias@example.com' })
    })

    await act(async () => {
      const send = [...view.container.querySelectorAll('button')].find((b) => b.textContent?.includes('Send'))!
      send.click()
      // The send waits for the queue; releasing it lets everything settle.
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    const order = bridge.calls.map((call) => call.command)
    const sent = order.indexOf('mail.send')
    const discarded = order.lastIndexOf('mail.discardDraft')
    expect(sent).toBeGreaterThan(-1)
    expect(discarded).toBeGreaterThan(sent)
    // Nothing may re-save the draft after it was discarded, or the sent message
    // reappears in Drafts.
    expect(order.slice(discarded).includes('mail.saveDraft')).toBe(false)
    // And the draft discarded is the one the saves actually created.
    const discardPayload = bridge.calls.filter((c) => c.command === 'mail.discardDraft').pop()!.payload
    expect(discardPayload.draft_id).toBe(savedDraftIds()[0])
  })

  it('saves the account and body as they stand when the queued save runs', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    render(<Composer tabId={tabId} />)

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    // Queued while the draft still belongs to account 'a'…
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'alias@example.com' })
    })
    // …but by the time it runs, the user has moved the draft to 'b'.
    await act(async () => {
      updateComposeDraft(tabId, { accountId: 'b', text: 'moved to b' })
    })
    await act(async () => {
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    const saves = bridge.calls.filter((c) => c.command === 'mail.saveDraft').map((c) => c.payload)
    // Only the save already in flight when the switch happened may carry 'a';
    // every later one is 'b', with the body that goes with it.
    expect(saves.filter((savedDraft) => savedDraft.account_id === 'a')).toHaveLength(1)
    const last = saves[saves.length - 1]
    expect(last.account_id).toBe('b')
    expect(last.body).toContain('moved to b')
  })

  it('does not attach an in-flight allocation to a newly selected account', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    render(<Composer tabId={tabId} />)

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      updateComposeDraft(tabId, { accountId: 'b', fromEmail: 'b@example.com' })
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    const saves = bridge.calls.filter((call) => call.command === 'mail.saveDraft').map((call) => call.payload)
    const allocatedForA = saves.find((save) => save.account_id === 'a')!.draft_id
    const savedForB = saves.filter((save) => save.account_id === 'b').pop()!
    expect(savedForB.draft_id).not.toBe(allocatedForA)
    expect(draftOf(tabId)?.draftMessageId).toBe(savedForB.draft_id)
    expect(
      bridge.calls.some(
        (call) =>
          call.command === 'mail.discardDraft' &&
          call.payload.account_id === 'a' &&
          call.payload.draft_id === allocatedForA,
      ),
    ).toBe(true)
  })

  it('cleans up the old account’s draft when the draft moves accounts', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    render(<Composer tabId={tabId} />)

    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
      await new Promise((resolve) => setTimeout(resolve, 20))
    })
    const allocatedForA = savedDraftIds()[0]
    expect(allocatedForA).toStartWith('allocated-')

    await act(async () => {
      updateComposeDraft(tabId, { accountId: 'b' })
      await new Promise((resolve) => setTimeout(resolve, 20))
    })

    // A's copy is deleted where it actually lives, and the tab stops pointing
    // at an id that belongs to another account's Drafts folder.
    const discards = bridge.calls.filter((c) => c.command === 'mail.discardDraft').map((c) => c.payload)
    expect(discards).toHaveLength(1)
    expect(discards[0]).toMatchObject({ account_id: 'a', draft_id: allocatedForA })
    expect(draftOf(tabId)?.draftMessageId).not.toBe(allocatedForA)
  })

  it('sends what is on screen, not what was there when Send was pressed', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'first draft' })!
    const view = render(<Composer tabId={tabId} />)

    // A save is in flight, so Send has to wait for it — and the composer stays
    // editable while it does.
    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })

    await act(async () => {
      const send = [...view.container.querySelectorAll('button')].find((b) => b.textContent?.includes('Send'))!
      send.click()
      updateComposeDraft(tabId, { text: 'second thoughts', to: 'y@example.com' })
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    const sent = bridge.calls.find((call) => call.command === 'mail.send')!.payload
    expect(sent.body).toContain('second thoughts')
    expect(sent.to).toBe('y@example.com')
  })

  it('revalidates recipient and account after autosaves drain', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const view = render(<Composer tabId={tabId} />)

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      const send = [...view.container.querySelectorAll('button')].find((button) =>
        button.textContent?.includes('Send'),
      )!
      send.click()
      updateComposeDraft(tabId, { to: '' })
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(bridge.calls.some((call) => call.command === 'mail.send')).toBe(false)
    expect(view.container.textContent).toContain('Add a recipient before sending')
  })

  it('revalidates the selected account after autosaves drain', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const view = render(<Composer tabId={tabId} />)

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      const send = [...view.container.querySelectorAll('button')].find((button) =>
        button.textContent?.includes('Send'),
      )!
      send.click()
      updateComposeDraft(tabId, { accountId: 'missing', fromEmail: 'missing@example.com' })
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(bridge.calls.some((call) => call.command === 'mail.send')).toBe(false)
    expect(view.container.textContent).toContain('Choose an available mail account')
  })

  it('asks for no-subject confirmation using the post-drain draft', async () => {
    const tabId = openComposeTab({ to: 'x@example.com', subject: 'Hello', text: 'hi' })!
    const view = render(
      <>
        <Composer tabId={tabId} />
        <AppConfirm />
      </>,
    )

    bridge.holdAllocation = () => {}
    await act(async () => {
      updateComposeDraft(tabId, { fromEmail: 'a@example.com' })
    })
    await act(async () => {
      const send = [...view.container.querySelectorAll('button')].find((button) =>
        button.textContent?.includes('Send'),
      )!
      send.click()
      updateComposeDraft(tabId, { subject: '' })
      bridge.holdAllocation?.()
      bridge.holdAllocation = null
      await new Promise((resolve) => setTimeout(resolve, 50))
    })

    expect(view.getByRole('alertdialog').textContent).toContain('Send this message without a subject?')
    expect(bridge.calls.some((call) => call.command === 'mail.send')).toBe(false)
    // Answer the question: an open confirm would block every later file's
    // shortcuts, as the app treats it as a modal.
    await act(async () => {
      settleConfirm(false)
    })
  })

  it('shows the new account’s signature in the editor when From changes', async () => {
    const tabId = openComposeTab()!
    render(<Composer tabId={tabId} />)
    expect(bodyHtml()).toContain('From A')

    await act(async () => {
      updateComposeDraft(tabId, { accountId: 'b', fromEmail: 'b@example.com' })
    })

    // The editor holds its own copy of the document; if it is not re-seeded the
    // message goes out under the previous account's signature.
    expect(bodyHtml()).toContain('From B')
    expect(bodyHtml()).not.toContain('From A')
  })

  it('keeps what the user typed when the signature is swapped under it', async () => {
    const tabId = openComposeTab()!
    render(<Composer tabId={tabId} />)

    await act(async () => {
      updateComposeDraft(tabId, { html: `<p>Hello there</p>${draftOf(tabId)!.html}` })
    })
    await act(async () => {
      updateComposeDraft(tabId, { accountId: 'b' })
    })

    expect(bodyHtml()).toContain('Hello there')
    expect(bodyHtml()).toContain('From B')
  })

  it('starts the caret above a quote seeded without a signature', () => {
    accounts$.set([{ ...account('a', ''), signature: { mode: 'none', html: '' } }])
    const tabId = openComposeTab({
      to: 'them@example.com',
      rich: false,
      text: '\n\n---------- Forwarded message ---------\nLunch?',
      signaturePlacement: 'aboveQuote',
    })!
    const view = render(<Composer tabId={tabId} />)

    const body = view.container.querySelector('textarea') as HTMLTextAreaElement
    expect(body.selectionStart).toBe(0)
  })

  it('unfolds a folded quote into the body, or drops it', async () => {
    accounts$.set([{ ...account('a', ''), signature: { mode: 'none', html: '' } }])
    const quote = { rich: false, html: '<p>On Mon, Them wrote:</p>', text: 'On Mon, Them wrote:\n> Lunch?' }
    const tabId = openComposeTab({ to: 'them@example.com', rich: false, text: 'Sounds good', quote })!
    const view = render(<Composer tabId={tabId} />)

    await act(async () => {
      view.getByTitle('Show quoted text').click()
    })
    expect(draftOf(tabId)?.text).toBe('Sounds good\n\nOn Mon, Them wrote:\n> Lunch?')
    expect(draftOf(tabId)?.quote).toBeUndefined()
    expect(view.queryByTitle('Show quoted text')).toBeNull()

    const other = openComposeTab({ to: 'them@example.com', rich: false, text: 'Sounds good', quote })!
    cleanup()
    const second = render(<Composer tabId={other} />)
    await act(async () => {
      fireEvent.contextMenu(second.getByTitle('Show quoted text'))
    })
    await act(async () => {
      second.getByText('Remove quoted text').click()
    })
    expect(draftOf(other)?.text).toBe('Sounds good')
    expect(draftOf(other)?.quote).toBeUndefined()
  })
})
