// Closes, reorders and restores desktop reader and compose tabs.
// The tab strip coordinates draft retention with the compose session.
// View preferences and tab selection follow the tab identity.

import { arrayMove } from '@dnd-kit/sortable'
import { isNarrowWindow } from '../lib/useNarrowWindow'
import type { ComposeDraft, Message, MessageTab } from '../types'
import { ui$ } from './ui'
import { accounts$ } from './accounts'
import { normalizeMessageId } from './mail'
import { resolveConversationMode, revealRemote, thread$ } from './thread'
import { closeComposeSession, forgetComposeSession, closeComposeSessionKeepingDraft } from './composeSessions'
import { compose$, newDraftMessageId } from './composeState'
import { discardClosedDraft } from './quickReply'
import { popToPreviousTab, activateConversationTab } from './readerTabs'

// Remove a tab once saves have drained; remote cleanup may still be running.
export function finishClosingMessageTab(id: string) {
  const tabs = compose$.tabs.get()
  const index = tabs.findIndex((tab) => tab.id === id)
  if (index === -1) return
  forgetComposeSession(id)
  const next = tabs.filter((tab) => tab.id !== id)
  compose$.tabs.set(next)
  if (compose$.activeTab.get() === id) {
    const target = popToPreviousTab(id, next)
    // Falling back to the Current tab needs a conversation to fall back to.
    // With none — a tab opened from tasks or a notification over an empty pane
    // — Current is not even offered in the strip (and in kanban view it would
    // close the pane), so hand over to a tab that's still open instead.
    const fallback = !target && !compose$.conversationThread.get() ? next[next.length - 1]?.id : ''
    const resolved = fallback || target
    const nextTab = resolved ? next.find((tab) => tab.id === resolved) : null
    if (!nextTab) {
      activateConversationTab()
    } else if (nextTab.kind === 'thread') {
      compose$.activeTab.set(nextTab.id)
      ui$.selectedThread.set(nextTab.threadId)
    } else {
      compose$.activeTab.set(nextTab.id)
    }
  }
}

// Bring a closed compose tab back as it was, or go to the tab already holding
// that draft: it may have been reopened from Drafts in the meantime.
export function reopenComposeTab(tab: MessageTab, compose: ComposeDraft) {
  if (isNarrowWindow()) ui$.contactsOpen.set(false)
  // The composer opens in the mail pane, which the calendar replaces.
  ui$.calendarOpen.set(false)
  const draftId = normalizeMessageId(compose.draftMessageId)
  const open = compose$.tabs
    .peek()
    .find((t) => t.id === tab.id || (t.compose && normalizeMessageId(t.compose.draftMessageId) === draftId))
  if (!open) compose$.tabs.set([...compose$.tabs.peek(), { ...tab, compose }])
  compose$.activeTab.set(open?.id ?? tab.id)
}

// Bring back a compose tab whose last save failed. The tab is the only copy of
// these edits, so it always comes back. If the draft was reopened from Drafts
// meanwhile, that tab holds an older copy under the same id: this one becomes a
// draft of its own, so neither can overwrite or discard the other.
export function restoreComposeTabAfterFailedSave(tab: MessageTab, compose: ComposeDraft) {
  ui$.calendarOpen.set(false)
  const draftId = normalizeMessageId(compose.draftMessageId)
  const held = compose$.tabs.peek().some((t) => t.compose && normalizeMessageId(t.compose.draftMessageId) === draftId)
  const restored = held ? { ...compose, draftMessageId: newDraftMessageId(), sourceDraft: undefined } : compose
  compose$.tabs.set([...compose$.tabs.peek(), { ...tab, compose: restored }])
  compose$.activeTab.set(tab.id)
}

// The single close entry point used by tab buttons, hotkeys and the palette.
// Mounted composers register their save queue here so no caller can bypass it.
export async function closeMessageTab(id: string) {
  const tabs = compose$.tabs.get()
  const index = tabs.findIndex((tab) => tab.id === id)
  if (index === -1) return
  if (tabs[index].kind !== 'compose') return finishClosingMessageTab(id)
  const draft = tabs[index].compose
  if (!draft) return finishClosingMessageTab(id)
  return closeComposeSession(id, async () => {
    const remoteId = draft.draftMessageId?.startsWith('local-draft-') ? undefined : draft.draftMessageId
    finishClosingMessageTab(id)
    if (remoteId || draft.sourceDraft) {
      await discardClosedDraft({
        threadId: draft.sourceDraft?.threadId ?? '',
        messageId: draft.sourceDraft?.messageId ?? '',
        folderId: draft.sourceDraft?.folderId ?? '',
        accountId: draft.accountId,
        draftMessageId: remoteId,
        // A reply escalated out of a conversation has no draft row of its
        // own to take the card away with it; the conversation's card stays
        // and only its Draft badge goes.
        replyThreadId: draft.sourceDraft ? '' : tabs[index].threadId,
      })
    }
  })
}

/**
 * The tab strip's ×, Ctrl+W and the palette's Close tab: a compose tab goes to Drafts (with an "Open
 * draft" toast), like Esc. Throwing a message away is the composer's own Discard button; a × that did
 * it silently lost work. A compose tab that isn't on screen has no mounted composer to do the last
 * save, so it is brought forward first.
 */
export async function closeMessageTabKeepingDraft(id: string) {
  const tab = compose$.tabs.peek().find((candidate) => candidate.id === id)
  if (tab?.kind !== 'compose') return closeMessageTab(id)
  if (closeComposeSessionKeepingDraft(id)) return
  compose$.activeTab.set(id)
  for (let attempt = 0; attempt < 10; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 30))
    if (closeComposeSessionKeepingDraft(id)) return
  }
  // No composer came up to save it: leave the tab open rather than lose it.
  console.error('Close tab: no composer to save the draft of', id)
}

// Close several tabs at once (the tab strip's close others/left/right/all).
// Compose tabs are skipped: closing one discards its draft, which a bulk close
// must never do behind the user's back.
export function closeMessageTabs(ids: string[]) {
  const wanted = new Set(ids)
  const closing = compose$.tabs
    .get()
    .filter((tab) => wanted.has(tab.id) && tab.kind !== 'compose')
    .map((tab) => tab.id)
  // Close the active tab last, so it falls back to a tab that stays open.
  const active = compose$.activeTab.get()
  for (const id of closing) if (id !== active) finishClosingMessageTab(id)
  if (closing.includes(active)) finishClosingMessageTab(active)
}

// Put a tab where another one sits, for reordering the strip. Either may have
// closed during the drag, in which case the order stands.
export function moveMessageTab(id: string, overId: string) {
  const tabs = compose$.tabs.get()
  const from = tabs.findIndex((tab) => tab.id === id)
  const to = tabs.findIndex((tab) => tab.id === overId)
  if (from === -1 || to === -1 || from === to) return
  compose$.tabs.set(arrayMove(tabs, from, to))
}

// Reveal a message's remote content in the conversation and on any reader tab
// already open for it. The tab needs its own copy because `resetThreadView`
// clears the conversation's reveal map on the next thread switch while the tab
// stays open — and an inactive tab is unmounted, so it cannot copy this itself.
export function revealMessageRemote(messageId: string) {
  revealRemote(messageId)
  compose$.tabs.set(
    compose$.tabs
      .get()
      .map((tab) => (tab.kind === 'reader' && tab.messageId === messageId ? { ...tab, revealRemote: true } : tab)),
  )
}

/** Keeps a reader tab's copy of its message in step with a change made from
 *  the tab, for when the thread it came from is no longer loaded. */
export function patchTabMessage(id: string, patch: Partial<Message>) {
  compose$.tabs.set(
    compose$.tabs
      .get()
      .map((tab) => (tab.id === id && tab.message ? { ...tab, message: { ...tab.message, ...patch } } : tab)),
  )
}

export function setTabViewMode(id: string, mode: 'html' | 'plain') {
  compose$.tabs.set(compose$.tabs.get().map((tab) => (tab.id === id ? { ...tab, viewMode: mode } : tab)))
}

/** Switches what's in front between HTML and plain text, for the shortcut:
 *  a reader tab flips its own view, a conversation flips its account's
 *  session override, as the header's "View as" menu does. A thread tab's
 *  account comes from the tab. Returns false when there was nothing to switch. */
export function toggleConversationMode(accountId: string | undefined): boolean {
  const activeTabId = compose$.activeTab.peek()
  const tab = activeTabId ? compose$.tabs.peek().find((candidate) => candidate.id === activeTabId) : undefined
  if (tab?.kind === 'reader') {
    // A message with no HTML part reads the same either way.
    if (!tab.bodyHtml) return false
    setTabViewMode(tab.id, tab.viewMode === 'html' ? 'plain' : 'html')
    return true
  }
  if (tab?.kind === 'compose') return false
  const id = tab?.kind === 'thread' && tab.accountId ? tab.accountId : accountId
  const accounts = accounts$.peek()
  if (!id || !accounts.some((account) => account.id === id)) return false
  const current = resolveConversationMode(accounts, thread$.conversationModeOverrides.peek(), id)
  thread$.conversationModeOverrides[id].set(current === 'html' ? 'plain' : 'html')
  return true
}
