// Coordinates desktop compose, reader and conversation tabs. The mailbox can start
// a reply, edit a draft or open a separate view through these actions.
// Callers include calendar.ts and kanban.ts.

import { t } from '../lib/i18n'
import { isNarrowWindow } from '../lib/useNarrowWindow'
import type { Account, Attachment, ComposeDraft, ComposerAttachment, Message } from '../types'
import { invoke } from '../lib/bridge'
import { CONVERSATION_PAGE_SIZE } from '../lib/pagination'
import { ui$, showToast } from './ui'
import { accounts$, isSendableAccount } from './accounts'
import { getActiveThread, normalizeMessageId } from './mail'
// From the leaf module: this one subscribes to it while loading, which can be
// inside an import cycle through mail.ts.
import { mail$ } from './mailState'
import { isDraftFolder } from './mailFolders'
import { parseMailto } from '../lib/mailto'
import {
  bodyWithSignature,
  bodyWithSwappedSignature,
  type ComposeBody,
  resolveSignature,
  signatureForms,
  type Signature,
  type SignaturePlacement,
  type SignatureTracking,
} from '../lib/signature'
import { settings$ } from './settings'
import { closeCurrentConversation, kanban$ } from './kanban'
import { draftSavesSettled, getComposeSession, pruneComposeSessions } from './composeSessions'
import {
  buildReplyRecipients,
  buildReplyThreading,
  detectAliasFrom,
  pickReplyTarget,
  replySubject,
  resolveQuickReplyFrom,
} from './composeReply'
import {
  compose$,
  COMPOSE_TABS_KEY,
  hasExtraComposeHeaders,
  newDraftMessageId,
  type PersistedComposeTab,
  type PersistedMailTab,
} from './composeState'
import {
  cancelQuickReplyDraftSave,
  clearQuickReplyDraftOwnership,
  discardingDraftIds,
  quickReplyAbandonedSaves,
  nextQuickReplyOwnershipTick,
  quickReplyDraftSaveInFlight,
  quickReplyWithoutSignature,
  reserveOpeningDraft,
  unconfirmedQuickReplyDraftId,
  seedQuickReplySignature,
} from './quickReply'
import { tabHistory, persistTabView, readerTab, newestMessage } from './readerTabs'
import {
  textToHtml,
  replyQuote,
  bodyHasReplyQuote,
  foldDraftQuote,
  forwardedHtmlBody,
  forwardedSubject,
  forwardedPlainBody,
} from './composeContent'
import { closeMessageTabs } from './composeTabClosing'
export { sendComposed, appendSentMessage, saveComposedDraft, draftIdOfFailedSave } from './composeDelivery'
export {
  finishClosingMessageTab,
  reopenComposeTab,
  restoreComposeTabAfterFailedSave,
  closeMessageTab,
  closeMessageTabKeepingDraft,
  closeMessageTabs,
  moveMessageTab,
  revealMessageRemote,
  patchTabMessage,
  setTabViewMode,
  toggleConversationMode,
} from './composeTabClosing'

export { escapeHtml, textToHtml, replyQuote, bodyHasReplyQuote, composedContent } from './composeContent'
export {
  activateConversationTab,
  pruneComposerMedia,
  openMessageTab,
  openThreadTab,
  newestMessage,
  openThreadTabById,
  openNotificationThread,
} from './readerTabs'

compose$.activeTab.onChange(({ value }) => {
  // Collapse immediate repeats so re-activating the current tab is a no-op.
  if (tabHistory[tabHistory.length - 1] === value) return
  tabHistory.push(value)
  if (tabHistory.length > 50) tabHistory.shift()
})

// Persist the tabs on every tab change. We only store the text portion of each
// compose draft — attachments are dropped because their base64 payloads can blow
// past localStorage's quota. On boot the tabs come back; the user reattaches
// files if needed. Thread and reader tabs are stored as references.
compose$.tabs.onChange(({ value: tabs }) => {
  pruneComposeSessions(new Set(tabs.map((tab) => tab.id)))
  const persisted = tabs.flatMap<PersistedComposeTab | PersistedMailTab>((t) => {
    if (t.kind !== 'compose') {
      const { id, kind, messageId, threadId, accountId, folderId, subject, from, fromAddr, feedIcon, viewMode } = t
      return [{ id, kind, messageId, threadId, accountId, folderId, subject, from, fromAddr, feedIcon, viewMode }]
    }
    if (!t.compose) return []
    return [{ id: t.id, subject: t.subject, compose: { ...t.compose, attachments: [] } }]
  })
  try {
    localStorage.setItem(COMPOSE_TABS_KEY, JSON.stringify(persisted))
  } catch {
    // localStorage quota exceeded — drop silently.
  }
})
compose$.activeTab.onChange(persistTabView)
compose$.conversationThread.onChange(persistTabView)

// Keep reader snapshots in sync with the loaded conversation: the body when
// background fetching completes, and the read and star flags whenever they
// change there, so the tab's menu still toggles the right way once that
// conversation is no longer loaded.
mail$.messages.onChange(({ value: messages }) => {
  const byId = new Map(messages.map((message) => [message.id, message]))
  for (const [index, tab] of compose$.tabs.peek().entries()) {
    if (tab.kind !== 'reader') continue
    const message = byId.get(tab.messageId)
    if (!message) continue
    if (tab.bodyMissing && !message.body_missing) {
      compose$.tabs[index].assign({
        body: message.body,
        bodyHtml: message.body_html,
        bodyMissing: false,
        attachments: message.attachments,
        // The bottom bar forwards from this: without the body it would send
        // an empty forward with none of the attachments the reader now shows.
        message,
      })
    } else if (tab.message && (tab.message.unread !== message.unread || tab.message.starred !== message.starred)) {
      compose$.tabs[index].assign({ message: { ...tab.message, unread: message.unread, starred: message.starred } })
    }
  }
})

// Put the persisted view back on screen. Call once at boot, after the kanban
// session: a board's conversation is its open card. The conversation behind a
// reader or compose tab stays unloaded until the user returns to it, so
// restoring never reads a thread that isn't on screen.
export function restoreTabSession() {
  const onBoard = !!kanban$.activeBoardId.peek()
  if (onBoard) {
    compose$.conversationThread.set(kanban$.paneThreadId.peek())
  } else {
    kanban$.paneThreadId.set('')
    kanban$.paneColumnKey.set('')
  }
  const active = compose$.tabs.peek().find((tab) => tab.id === compose$.activeTab.peek())
  const conversation = compose$.conversationThread.peek()
  if (active?.kind === 'thread') ui$.selectedThread.set(active.threadId)
  else if (!active && conversation) ui$.selectedThread.set(conversation)
  else if (!active) closeCurrentConversation()
  if (compose$.activeTab.peek() || conversation) ui$.mobilePane.set('conversation')
  void reconcileRestoredTabs()
}

async function restoredThreadPage(threadId: string, cursor: string) {
  return invoke<{ messages: Message[]; next_cursor?: string }>('mail.threadRead', {
    thread_id: threadId,
    limit: CONVERSATION_PAGE_SIZE,
    ...(cursor ? { before_cursor: cursor } : {}),
  })
}

// Check each restored conversation against the mailbox, one at a time: anything
// deleted or moved while the app was closed closes, and reader tabs get their
// bodies back. A failed read proves nothing (offline, say), so it closes nothing.
async function reconcileRestoredTabs() {
  const threadIds = new Set(
    compose$.tabs
      .peek()
      .filter((tab) => tab.kind !== 'compose')
      .map((tab) => tab.threadId),
  )
  if (compose$.conversationThread.peek()) threadIds.add(compose$.conversationThread.peek())
  for (const threadId of threadIds) {
    try {
      const wanted = new Set(
        compose$.tabs
          .peek()
          .filter((tab) => tab.kind === 'reader' && tab.threadId === threadId)
          .map((tab) => tab.messageId),
      )
      const found = new Map<string, Message>()
      const cursors = new Set<string>()
      let cursor = ''
      let empty = true
      do {
        cursors.add(cursor)
        const page = await restoredThreadPage(threadId, cursor)
        if (page.messages.length > 0) empty = false
        for (const message of page.messages) if (wanted.has(message.id)) found.set(message.id, message)
        cursor = page.next_cursor ?? ''
      } while (cursor && !cursors.has(cursor) && found.size < wanted.size)

      const tabs = compose$.tabs.peek()
      closeMessageTabs(
        tabs
          .filter((tab) => tab.threadId === threadId && (empty || (tab.kind === 'reader' && !found.has(tab.messageId))))
          .map((tab) => tab.id),
      )
      if (empty && compose$.conversationThread.peek() === threadId) closeCurrentConversation()
      for (const [index, tab] of compose$.tabs.peek().entries()) {
        const message = tab.kind === 'reader' ? found.get(tab.messageId) : undefined
        if (message) compose$.tabs[index].assign({ ...readerTab(message), viewMode: tab.viewMode })
      }
    } catch (error) {
      console.error('restoring tab failed', error)
    }
  }
}

/**
 * The signature an account sends, in both body forms. Both are resolved even for
 * a rich draft: the composer can be toggled to plaintext at any time, and a
 * half-tracked signature could then be neither found nor replaced.
 */
export function resolveSignatureFor(account: Account | undefined): Signature {
  return signatureForms(account ? resolveSignature(account, settings$.signature.peek()) : '')
}

/**
 * Move a draft to another From account, swapping the signature it carries for
 * the new account's. Sending account B's mail under account A's signature is
 * worse than no signature at all, so this runs on every account change; an
 * edited signature is left alone (see `bodyWithSwappedSignature`).
 */
function withSignatureForAccount(draft: ComposeDraft, partial: Partial<ComposeDraft>): Partial<ComposeDraft> {
  if (!partial.accountId || partial.accountId === draft.accountId) return partial
  const account = accounts$.peek().find((acc) => acc.id === partial.accountId)
  // Swap over the draft as this update leaves it, so a body supplied in the
  // same call is what gets the new signature — not the body it replaced.
  const updated = { ...draft, ...partial }
  const swapped = bodyWithSwappedSignature(updated, draft.signature, resolveSignatureFor(account))
  return { ...partial, html: swapped.body.html, text: swapped.body.text, signature: swapped.tracking }
}

let composeSeq = 0

// Open a full-pane compose/reply editor as a new tab. Returns silently if no
// account can send mail. `seed` pre-fills a reply (recipient, subject, body…).
type ComposeSeed = Partial<ComposeDraft> & {
  title?: string
  threadId?: string
  /** Where the signature lands; 'aboveQuote' for a seeded quote (forwards). */
  signaturePlacement?: SignaturePlacement
  /**
   * The message a reply answers, quoted under the seeded body (see
   * {@link replyQuote}) but folded away until the user unfolds it. An HTML
   * original makes the draft rich.
   */
  quote?: ComposeBody
  /**
   * Skip the signature. Set when the body is an existing message being re-opened
   * (a saved draft, "Edit as New Message") — it already carries whatever
   * signature it was written with, and a second copy is not wanted.
   */
  noSignature?: boolean
}

export function openComposeTab(seed?: ComposeSeed): string | undefined {
  if (isNarrowWindow()) ui$.contactsOpen.set(false)
  const sendable = accounts$.get().filter(isSendableAccount)
  if (sendable.length === 0) return undefined
  // The composer opens in the mail pane, which the calendar replaces.
  ui$.calendarOpen.set(false)
  const selected = ui$.selectedAccount.get()
  // A seed from a thread in local folders names an account that cannot send;
  // the reply then goes out from the selected or first sendable account.
  const seededAccountId = sendable.some((acc) => acc.id === seed?.accountId) ? seed?.accountId : undefined
  const accountId = seededAccountId ?? sendable.find((acc) => acc.id === selected)?.id ?? sendable[0].id
  // Only a truly blank compose (no seeded body) should pick up the account's
  // HTML/plain preference; reply/forward/mailto paths that seed plain `text`
  // stay plain since the editor only ever hydrates from `html`.
  const hasSeededBody = !!(seed?.text || seed?.html)
  const account = sendable.find((acc) => acc.id === accountId)

  // The signature is inserted into the body up front so it is editable (and
  // visible) like the rest of the draft, rather than appearing at send time.
  const quote = seed?.quote
  const rich = seed?.rich ?? (hasSeededBody ? false : (account?.conversation_html ?? true))
  // An HTML original is quoted as HTML, so the reply goes rich with it.
  const seeded =
    quote?.rich && !rich
      ? { rich: true, html: seed?.html || textToHtml(seed?.text ?? ''), text: '' }
      : { rich, html: seed?.html ?? '', text: seed?.text ?? '' }
  // A body this app did not compose (a saved draft, "Edit as New Message") may
  // already carry a signature, so it stays unmanaged: `undefined`, not `null`.
  const placement = seed?.signaturePlacement ?? 'belowText'
  const signature = seed?.noSignature ? undefined : resolveSignatureFor(account)
  // The placement is recorded even when the account sends no signature, so a
  // forward that later moves to an account with one still puts it above the
  // quote rather than after it.
  const tracking: SignatureTracking = signature ? { ...signature, placement } : undefined
  const body = signature ? bodyWithSignature(seeded, signature, placement) : seeded

  const draft: ComposeDraft = {
    accountId,
    fromEmail: seed?.fromEmail ?? '',
    to: seed?.to ?? '',
    cc: seed?.cc ?? '',
    bcc: seed?.bcc ?? '',
    replyTo: seed?.replyTo ?? '',
    subject: seed?.subject ?? '',
    ...body,
    showCcBcc:
      seed?.showCcBcc ??
      hasExtraComposeHeaders({
        cc: seed?.cc ?? '',
        bcc: seed?.bcc ?? '',
      }),
    inReplyTo: seed?.inReplyTo ?? '',
    references: seed?.references ?? '',
    draftMessageId: seed?.draftMessageId ?? newDraftMessageId(),
    signature: tracking,
    quote: quote ? { html: quote.html, text: quote.text } : undefined,
    sourceDraft: seed?.sourceDraft,
    attachments: seed?.attachments ?? [],
  }
  const id = `compose-${Date.now()}-${composeSeq++}`
  compose$.tabs.push({
    id,
    kind: 'compose',
    messageId: '',
    threadId: seed?.threadId ?? '',
    subject: seed?.title || draft.subject || 'New message',
    from: '',
    body: '',
    viewMode: 'plain',
    compose: draft,
  })
  compose$.activeTab.set(id)
  return id
}

// Escalate the active thread's quick reply into a full-window composer tab,
// seeded as a reply (recipients, "Re:" subject, threading headers) and carrying
// over whatever's been typed/attached. Clears the quick reply on success.
// No-op when there's no active conversation. `replyAll` seeds the wider
// recipient list; the quick reply box has no recipient fields to show it in, so
// reply-all always lands here rather than in the box.
export function openReplyInFullEditor(options?: { replyAll?: boolean }) {
  const t = getActiveThread()
  if (!t) return
  const target = pickReplyTarget(t)
  const subject = replySubject(t, target)
  const accounts = accounts$.get()
  const { to, cc } = buildReplyRecipients(target, options?.replyAll)
  const { in_reply_to, references } = buildReplyThreading(target)
  const replyAcc = accounts.find((acc) => acc.id === t.account_id)
  // Hand off any draft already saved for this quick reply so the full editor
  // continues editing the same server-side draft instead of creating a
  // duplicate one — or the id a failed save may have left a copy under, which
  // the editor's next save then overwrites instead of stranding.
  const existingDraftId =
    (compose$.quickReplyDraftSaved.peek() ? compose$.quickReplyDraftId.peek() : unconfirmedQuickReplyDraftId()) ||
    undefined
  // Nothing to hand over while the first autosave is still allocating its id, so
  // the tab starts a draft of its own — mark the copy that save lands as
  // abandoned rather than leaving it in Drafts with nothing pointing at it.
  if (!existingDraftId && quickReplyDraftSaveInFlight) {
    quickReplyAbandonedSaves.set(t.thread_id, nextQuickReplyOwnershipTick())
  }
  cancelQuickReplyDraftSave()
  // The seeded signature is handed over stripped, so the full composer inserts
  // and tracks its own copy — the account is the same, so this is the identical
  // text, now swappable if the draft later changes identity. When it can't be
  // found the user has written into it: it stays in the body as theirs, and the
  // composer is told not to add a second.
  const carried = quickReplyWithoutSignature()
  // A bar holding a saved quoted reply already carries its quote.
  const quote = replyQuote(target)
  openComposeTab({
    accountId: t.account_id || undefined,
    fromEmail: resolveQuickReplyFrom(target, replyAcc),
    to,
    cc,
    showCcBcc: !!cc.trim(),
    subject,
    text: carried.text,
    noSignature: !carried.found && !!compose$.quickReplySignature.peek()?.text,
    attachments: compose$.composerAttachments.get(),
    inReplyTo: in_reply_to,
    references,
    draftMessageId: existingDraftId,
    title: subject,
    threadId: t.thread_id,
    quote: quote && !bodyHasReplyQuote(carried.text, quote) ? quote : undefined,
  })
  compose$.composerAttachments.set([])
  clearQuickReplyDraftOwnership()
  compose$.quickReplyFrom.set('')
  // The thread stays open behind the new tab, so the box the user comes back to
  // is a fresh quick reply — signature and all.
  seedQuickReplySignature()
}

export function openMailtoCompose(raw: string) {
  const draft = parseMailto(raw)
  if (!draft) return
  if (accounts$.get().filter(isSendableAccount).length === 0) {
    showToast(t('compose.toast.addMailAccountBeforeComposing'))
    return
  }
  openComposeTab({
    to: draft.to,
    cc: draft.cc,
    bcc: draft.bcc,
    subject: draft.subject,
    text: draft.body,
    showCcBcc: !!draft.cc || !!draft.bcc,
    title: draft.subject || 'New message',
  })
}

// Open a message as a brand-new editable draft ("Edit as New Message", à la
// Apple Mail / Thunderbird). Works on any message regardless of folder — it's a
// duplicate-into-compose, not a reply: the subject/recipients/body/attachments
// are copied and the user edits from there. Deliberately carries NO In-Reply-To/
// References, so the copy starts a fresh conversation instead of threading into
// the original.
//
// Body: the original HTML is carried into the rich editor when present (else the
// plaintext body). Attachments live in the media cache as files keyed by
// `/media/<key>`; we read their bytes back as base64 composer attachments.
// Inline images (referenced as `/media/<key>` inside the carried HTML) are NOT
// re-attached here — the composer's send path re-inlines them from those refs,
// so adding them again would duplicate. Only genuine, non-inline attachments are
// pulled in as file chips.
export async function editAsNewMessage(message: Message) {
  if (accounts$.get().filter(isSendableAccount).length === 0) {
    showToast(t('compose.toast.addMailAccountBeforeComposing'))
    return
  }
  const rich = !!message.body_html
  const html = message.body_html ?? ''
  const id = openComposeTab({
    accountId: message.account_id || undefined,
    to: message.to ?? '',
    cc: message.cc ?? '',
    showCcBcc: !!message.cc?.trim(),
    subject: message.subject ?? '',
    rich,
    html: rich ? html : '',
    text: rich ? '' : (message.body ?? ''),
    title: message.subject || 'New message',
    noSignature: true,
  })
  if (!id) return

  await fetchComposeTabAttachments(id, message.attachments ?? [], rich ? html : '')
}

// Fill a tab just opened with a message's attachments. Until they land the
// session holds the fetch, so an Esc save can wait for them: it would otherwise
// keep the message without its attachments, and a saved draft replaced with
// that copy would lose them for good. The tab may have been edited or closed
// meanwhile; bail if it's gone, otherwise merge onto what it now holds.
async function fetchComposeTabAttachments(id: string, attachments: Attachment[], inlineHtml: string) {
  const session = getComposeSession(id)
  const fetch = readComposerAttachments(attachments, inlineHtml)
  session.attachmentFetches.add(fetch)
  const valid = await fetch.finally(() => session.attachmentFetches.delete(fetch))
  if (valid.length === 0) return
  const tab = compose$.tabs.get().find((t) => t.id === id)
  if (!tab?.compose) return
  updateComposeDraft(id, { attachments: [...tab.compose.attachments, ...valid] })
}

function composeFromDraftMessage(message: Message): ComposeSeed {
  const body = foldDraftQuote(message)
  return {
    accountId: message.account_id || undefined,
    fromEmail: message.from_addr ?? '',
    to: message.to ?? '',
    cc: message.cc ?? '',
    bcc: message.bcc ?? '',
    replyTo: message.reply_to ?? '',
    showCcBcc: hasExtraComposeHeaders({ cc: message.cc ?? '', bcc: message.bcc ?? '' }),
    subject: message.subject ?? '',
    rich: body.rich,
    html: body.html,
    text: body.text,
    quote: body.quote && { rich: body.rich, ...body.quote },
    inReplyTo: '',
    references: message.references ?? '',
    // Not an id already being expunged: that copy is going away, so this tab's
    // content belongs in a draft of its own.
    draftMessageId:
      message.message_id && !discardingDraftIds.has(normalizeMessageId(message.message_id))
        ? message.message_id
        : newDraftMessageId(),
    sourceDraft: {
      threadId: message.thread_id,
      messageId: message.id,
      folderId: message.folder_id,
    },
    title: message.subject || 'New message',
    noSignature: true,
  }
}

/** Whether a compose draft is the one this Drafts row holds a copy of. */
function composeHoldsDraft(compose: ComposeDraft, draft: Message): boolean {
  const source = compose.sourceDraft
  return (
    (!!draft.message_id && normalizeMessageId(compose.draftMessageId) === normalizeMessageId(draft.message_id)) ||
    source?.messageId === draft.id ||
    (source?.threadId === draft.thread_id && source?.folderId === draft.folder_id)
  )
}

function activateOpenDraftCompose(draft: Message): boolean {
  const existing = compose$.tabs
    .get()
    .find((tab) => tab.kind === 'compose' && !!tab.compose && composeHoldsDraft(tab.compose, draft))
  if (!existing) return false
  compose$.activeTab.set(existing.id)
  return true
}

/**
 * Before opening a Drafts row, let this draft's Esc save finish. The row can be the copy that save is
 * replacing, and a tab opened from it would autosave the older text over the newer. The tab comes back
 * as it closed, as the store may not hold the new copy yet; when the save failed, the tab is already
 * back. True when a tab now holds the draft.
 */
async function reopenAfterDraftSaves(draft: Message): Promise<boolean> {
  const [saved] = await draftSavesSettled((compose) => composeHoldsDraft(compose, draft))
  saved?.reopen()
  return !!saved || activateOpenDraftCompose(draft)
}

export function draftShouldOpenConversation(messages: Message[], draft: Message): boolean {
  return (
    messages.some((message) => !isDraftFolder(message.folder_id, message.account_id)) ||
    !!draft.references?.trim() ||
    !!draft.original_thread_id?.trim()
  )
}

async function openDraftMessageInCompose(draft: Message) {
  if (activateOpenDraftCompose(draft)) return true
  const id = openComposeTab(composeFromDraftMessage(draft))
  if (!id) return true

  await fetchComposeTabAttachments(id, draft.attachments ?? [], draft.body_html ?? '')
  return true
}

// Restore a saved server-side Drafts row into the full composer. Drafts are
// stored as normal IMAP messages, but clicking one should resume editing rather
// than open a read-only conversation.
export async function openDraftCompose(thread: Message) {
  if (!isDraftFolder(thread.folder_id, thread.account_id)) return false
  if (accounts$.get().filter(isSendableAccount).length === 0) {
    showToast(t('compose.toast.addMailAccountBeforeComposing'))
    return true
  }

  if (activateOpenDraftCompose(thread)) return true

  const release = reserveOpeningDraft(thread)
  try {
    if (await reopenAfterDraftSaves(thread)) return true
    const result = await invoke<{ messages: Message[] }>('mail.threadRead', {
      thread_id: thread.thread_id,
      limit: CONVERSATION_PAGE_SIZE,
    })
    const draft =
      newestMessage(
        (result.messages ?? []).filter((message) => isDraftFolder(message.folder_id, message.account_id)),
      ) ?? thread
    // The read can resolve to a copy under a different id; hold that one too for
    // the (synchronous) hop to the tab that now owns it.
    const releaseResolved = reserveOpeningDraft(draft)
    try {
      await openDraftMessageInCompose(draft)
    } finally {
      releaseResolved()
    }
  } catch (error) {
    showToast(error instanceof Error ? error.message : t('compose.toast.couldNotOpenDraft'), 'error')
  } finally {
    release()
  }
  return true
}

export async function openDraftConversationOrCompose(thread: Message) {
  if (!isDraftFolder(thread.folder_id, thread.account_id)) return false

  const release = reserveOpeningDraft(thread)
  try {
    if (await reopenAfterDraftSaves(thread)) return true
    const result = await invoke<{ messages: Message[] }>('mail.threadRead', {
      thread_id: thread.thread_id,
      limit: CONVERSATION_PAGE_SIZE,
    })
    const messages = result.messages ?? []
    const draft =
      newestMessage(messages.filter((message) => isDraftFolder(message.folder_id, message.account_id))) ?? thread

    if (draftShouldOpenConversation(messages, draft)) {
      // This one opens a conversation, not an editor: hand the draft to the
      // quick reply by releasing before the messages land, or the hold that was
      // protecting it would suppress the very hydration that takes it over.
      release()
      compose$.activeTab.set('')
      ui$.selectedThread.set(thread.thread_id)
      ui$.mobilePane.set('conversation')
      if (ui$.selectedThread.peek() === thread.thread_id) {
        mail$.messages.set(messages)
        mail$.messagesCursor.set('')
        mail$.messagesCursorThread.set(thread.thread_id)
        mail$.messagesLoadingMore.set(false)
        mail$.threadLoading.set(false)
      }
      return true
    }

    if (accounts$.get().filter(isSendableAccount).length === 0) {
      showToast(t('compose.toast.addMailAccountBeforeComposing'))
      return true
    }
    const releaseResolved = reserveOpeningDraft(draft)
    try {
      await openDraftMessageInCompose(draft)
    } finally {
      releaseResolved()
    }
  } catch (error) {
    showToast(error instanceof Error ? error.message : t('compose.toast.couldNotOpenDraft'), 'error')
  } finally {
    release()
  }
  return true
}

// Open a clean compose draft for forwarding a message. Recipients and threading
// headers are intentionally blank; the original content is quoted in the body
// and non-inline attachments are copied back into the composer.
/** Reply-all to one message, rather than to the conversation's reply target:
 * the message menu acts on the message under the cursor. Opens a composer tab
 * — the recipients are the point of the action, and only the full editor shows
 * them. The quick reply's own draft is left alone; this is a separate reply. */
export function replyAllToMessage(message: Message) {
  replyToMessage(message, true)
}

/** Reply (or reply-all) to one message in a compose tab, rather than to the
 *  conversation's reply target in the quick reply bar. */
export function replyToMessage(message: Message, replyAll = false) {
  const accounts = accounts$.get()
  if (accounts.filter(isSendableAccount).length === 0) {
    showToast(t('compose.toast.addMailAccountBeforeComposing'))
    return
  }
  const { to, cc } = buildReplyRecipients(message, replyAll)
  const { in_reply_to, references } = buildReplyThreading(message)
  const acc = accounts.find((a) => a.id === message.account_id)
  const subject = message.subject.startsWith('Re:') ? message.subject : `Re: ${message.subject}`
  openComposeTab({
    threadId: message.thread_id,
    accountId: message.account_id || undefined,
    fromEmail: acc ? detectAliasFrom(message, acc) : '',
    to,
    cc,
    showCcBcc: !!cc.trim(),
    subject,
    title: subject,
    inReplyTo: in_reply_to,
    references,
    quote: replyQuote(message),
  })
}

export async function forwardMessage(message: Message) {
  if (accounts$.get().filter(isSendableAccount).length === 0) {
    showToast(t('compose.toast.addMailAccountBeforeComposing'))
    return
  }
  const rich = !!message.body_html
  const html = rich ? forwardedHtmlBody(message) : ''
  const id = openComposeTab({
    accountId: message.account_id || undefined,
    to: '',
    cc: '',
    bcc: '',
    showCcBcc: false,
    subject: forwardedSubject(message.subject),
    rich,
    html,
    text: rich ? '' : forwardedPlainBody(message),
    title: forwardedSubject(message.subject),
    signaturePlacement: 'aboveQuote',
  })
  if (!id) return

  await fetchComposeTabAttachments(id, message.attachments ?? [], html)
}

export async function readComposerAttachments(
  attachments: Attachment[],
  inlineHtml: string,
): Promise<ComposerAttachment[]> {
  const toFetch = attachments.filter((a) => a.key && !inlineHtml.includes(`/media/${a.key}`))
  if (toFetch.length === 0) return []

  const fetched = await Promise.all(
    toFetch.map(async (a) => {
      try {
        const response = await invoke<{ data: string; mime: string; size: number }>('mail.readAttachment', {
          key: a.key,
        })
        if (!response?.data) return null
        const att: ComposerAttachment = {
          id: `${Date.now()}-${Math.random().toString(36).substring(2, 9)}`,
          filename: a.filename,
          mime: a.mime || response.mime || 'application/octet-stream',
          size: a.size || response.size,
          data: response.data,
        }
        return att
      } catch {
        return null
      }
    }),
  )
  return fetched.filter((a): a is ComposerAttachment => a !== null)
}

// Merge a partial draft into a compose tab, keeping the tab label in sync with
// the subject line.
export function updateComposeDraft(id: string, partial: Partial<ComposeDraft>) {
  compose$.tabs.set(
    compose$.tabs.get().map((tab) => {
      if (tab.id !== id || !tab.compose) return tab
      const compose = { ...tab.compose, ...withSignatureForAccount(tab.compose, partial) }
      const subject = partial.subject !== undefined ? partial.subject.trim() || 'New message' : tab.subject
      return { ...tab, compose, subject }
    }),
  )
}
