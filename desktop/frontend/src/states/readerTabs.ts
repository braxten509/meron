// Opens desktop conversation tabs and restores their saved reading position.
// The reader resolves notifications, message selections and restored tabs to a thread.
// Closing or reopening a tab uses its recorded navigation history.

import { t } from '../lib/i18n'
import { isNarrowWindow } from '../lib/useNarrowWindow'
import type { Message, MessageTab } from '../types'
import { invoke } from '../lib/bridge'
import { CONVERSATION_PAGE_SIZE } from '../lib/pagination'
import { senderThreadIdLike } from '../lib/senderThread'
import { ui$, showToast } from './ui'
import { accounts$, accountIdentities } from './accounts'
import { mail$ } from './mailState'
import { thread$ } from './thread'
import { compose$, TAB_VIEW_KEY } from './composeState'

// Return to the Current conversation tab, restoring the thread it was showing
// before a thread tab took over selectedThread.
export function activateConversationTab() {
  ui$.selectedThread.set(compose$.conversationThread.peek())
  compose$.activeTab.set('')
}

// Activation trail of tab ids ("" = Current), oldest first. Closing the active
// tab walks this back to the tab you were on before opening it — e.g. opening a
// message in a new tab from a thread tab and closing it returns to that thread
// tab, not whatever happens to sit next in the strip.
export const tabHistory: string[] = []

// Drop the closed tab from the trail, then return the most recent entry that's
// still a live tab (or "" for Current). Pops dead entries as it goes.
export function popToPreviousTab(closedId: string, remaining: MessageTab[]): string {
  for (let i = tabHistory.length - 1; i >= 0; i--) {
    if (tabHistory[i] === closedId) tabHistory.splice(i, 1)
  }
  while (tabHistory.length > 0) {
    const candidate = tabHistory[tabHistory.length - 1]
    if (candidate === '' || remaining.some((tab) => tab.id === candidate)) return candidate
    tabHistory.pop()
  }
  return ''
}

// Delete orphaned inline-image files from earlier sessions. writeMediaFile
// writes one loose file per inline paste into the media root and nothing ever
// reclaims them, so a discarded or sent draft leaks its images. On boot we
// collect the `/media/<key>` keys still referenced by the rehydrated compose
// tabs and let the backend remove every other loose file. Fire-and-forget.
export function pruneComposerMedia() {
  const keys = new Set<string>()
  // Match a root-level media key (no slash) — exactly writeMediaFile's output.
  // Per-account refs look like /media/<account>/… and are skipped by the regex
  // (and ignored by the backend, which only deletes regular files at the root).
  const re = /\/media\/([^/"'\s)>\\]+)/g
  for (const tab of compose$.tabs.get()) {
    const html = tab.compose?.html
    if (!html) continue
    for (const match of html.matchAll(re)) keys.add(match[1])
  }
  invoke('composer.pruneMedia', { keys: [...keys] }).catch(() => {})
}

export function persistTabView() {
  try {
    localStorage.setItem(
      TAB_VIEW_KEY,
      JSON.stringify({
        activeTab: compose$.activeTab.peek(),
        conversationThread: compose$.conversationThread.peek(),
      }),
    )
  } catch {
    // localStorage quota exceeded — drop silently.
  }
}

// Open a single message in its own reader tab. The HTML is already on the
// message (shipped with threadRead), so this is instant — no fetch. Re-opening
// an already-open message just re-activates its tab.
// What the tab strip's avatar needs, as ThreadListItem draws it: a feed item
// shows its feed's icon rather than resolving the sender address.
function tabAvatar(message: Message): Pick<MessageTab, 'fromAddr' | 'feedIcon'> {
  if (message.feed_url) return { feedIcon: message.feed_icon }
  return { fromAddr: message.from_addr }
}

export function readerTab(message: Message): MessageTab {
  const account = accounts$.get().find((acc) => acc.id === message.account_id)
  const preferHtml = account?.conversation_html ?? true
  return {
    id: message.id,
    kind: 'reader',
    messageId: message.id,
    threadId: message.thread_id,
    // Carried so the tab can resolve this message's remote-content policy.
    accountId: message.account_id,
    revealRemote: !!thread$.revealedRemote.peek()[message.id],
    // Snapshotted at open time: the From address alone misses an alias or a
    // send still in flight (the same rule useMessageView applies in the
    // conversation).
    outgoing:
      !!message.send_status ||
      message.outgoing === true ||
      (!!account &&
        accountIdentities(account).some(
          (identity) => identity.email.trim().toLowerCase() === message.from_addr.trim().toLowerCase(),
        )),
    subject: message.subject || '(no subject)',
    from: message.from_name || message.from_addr,
    ...tabAvatar(message),
    fromRaw: message.from_name ? `${message.from_name} <${message.from_addr}>` : message.from_addr,
    to: message.to,
    cc: message.cc,
    bcc: message.bcc,
    replyTo: message.reply_to,
    date: message.date,
    body: message.body,
    bodyHtml: message.body_html,
    bodyMissing: message.body_missing,
    attachments: message.attachments,
    viewMode: message.body_html && preferHtml ? 'html' : 'plain',
    message,
  }
}

export function openMessageTab(message: Message) {
  if (isNarrowWindow()) ui$.contactsOpen.set(false)
  const existing = compose$.tabs.get().find((tab) => tab.messageId === message.id)
  if (existing) {
    compose$.activeTab.set(existing.id)
    return
  }
  const tab = readerTab(message)
  compose$.tabs.push(tab)
  compose$.activeTab.set(tab.id)
}

// Open a conversation itself as a tab. The tab stores enough thread metadata to
// render even when the current mailbox/rail selection does not contain it.
export function openThreadTab(thread: Message) {
  if (isNarrowWindow()) ui$.contactsOpen.set(false)
  // The conversation shows in the mail pane, which the calendar replaces —
  // even when its tab is the active one already, so no tab change says so.
  ui$.calendarOpen.set(false)
  const id = `thread-${thread.thread_id}`
  const existing = compose$.tabs.get().find((tab) => tab.id === id)
  if (!existing) {
    const tab: MessageTab = {
      id,
      kind: 'thread',
      messageId: '',
      threadId: thread.thread_id,
      accountId: thread.account_id,
      folderId: thread.folder_id,
      subject: thread.subject || '(no subject)',
      from: thread.from_name || thread.from_addr,
      ...tabAvatar(thread),
      body: '',
      viewMode: 'plain',
    }
    compose$.tabs.push(tab)
  }
  // Activate the tab before retargeting selectedThread so the conversationThread
  // mirror skips this change and the Current tab keeps its own thread.
  compose$.activeTab.set(id)
  ui$.selectedThread.set(thread.thread_id)
  ui$.mobilePane.set('conversation')
}

export function newestMessage(messages: Message[]): Message | null {
  return messages.reduce<Message | null>((newest, message) => {
    if (!newest) return message
    return message.date > newest.date ? message : newest
  }, null)
}

/**
 * Open a thread in its own tab from an id alone — a notification tap, a task's
 * mail link. `notFoundMessage` is the toast for a thread the read cannot find
 * (deleted, expunged), since each caller knows what the user was pointing at.
 */
export async function openThreadTabById(
  threadId: string,
  notFoundMessage = t('compose.toast.couldNotOpenNotificationThread'),
) {
  if (!threadId) return
  if (isNarrowWindow()) ui$.contactsOpen.set(false)
  // The conversation shows in the mail pane, which the calendar replaces —
  // even when its tab is the active one already, so no tab change says so.
  ui$.calendarOpen.set(false)
  const id = `thread-${threadId}`
  const existing = compose$.tabs.get().find((tab) => tab.id === id)
  if (existing) {
    compose$.activeTab.set(id)
    ui$.selectedThread.set(threadId)
    ui$.mobilePane.set('conversation')
    return
  }

  try {
    const result = await invoke<{ messages: Message[] }>('mail.threadRead', {
      thread_id: threadId,
      limit: CONVERSATION_PAGE_SIZE,
    })
    const message = newestMessage(result.messages ?? [])
    if (!message) {
      showToast(notFoundMessage, 'error')
      return
    }
    openThreadTab(message)
    // Seed the conversation with the messages we just fetched so the reply target
    // (and its Message-ID) is available immediately. Without this, a reply sent
    // before the selectedThread effect re-fetches falls back to the thread card,
    // which carries no Message-ID, and the reply lands unthreaded.
    if (ui$.selectedThread.get() === message.thread_id) {
      mail$.messages.set(result.messages ?? [])
    }
  } catch (error) {
    showToast(error instanceof Error ? error.message : notFoundMessage, 'error')
  }
}

/**
 * Open what a new-mail notification points at. With group by sender on, that
 * is the sender's chat in the same folder rather than the thread, built from
 * the notified message's own sender address (`fromAddr`). Falls back to the
 * thread when the address is missing.
 */
export async function openNotificationThread(threadId: string, fromAddr?: string) {
  if (!threadId) return
  if (ui$.groupBySender.peek() && fromAddr && !threadId.includes('#rss#')) {
    const senderId = senderThreadIdLike(threadId, fromAddr)
    if (senderId) {
      await openThreadTabById(senderId)
      return
    }
  }
  await openThreadTabById(threadId)
}
