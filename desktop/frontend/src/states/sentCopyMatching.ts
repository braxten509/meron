// Reconciles locally sent bubbles with refreshed server messages.
// The open thread retains its optimistic rows until a matching server copy arrives.
// Envelope fields and a bounded time window handle copies without the same Message-ID.

import type { Message } from '../types'
import { isLocalSendId } from './pendingSends'
import { bareAddr, splitAddressList } from '../lib/address'
import { isDraftFolder } from './mailFolders'

/**
 * Keep optimistic sends visible while the server's Sent copy catches up. Some providers expose it over
 * IMAP only after SMTP has returned (Proton Bridge can take several seconds), so replacing the thread
 * page wholesale creates a gap where the reply disappears. Once the canonical row arrives, it replaces
 * the local bubble instead of rendering twice.
 */
export function mergeRefreshedThreadMessages(current: Message[], refreshed: Message[], threadId: string): Message[] {
  const canonicalMessageIds = new Set(
    refreshed.map((message) => normalizeMessageId(message.message_id)).filter(Boolean),
  )
  const unresolved = current.filter((message) => {
    if (message.thread_id !== threadId || !isLocalSendId(message.id)) return false
    const messageId = normalizeMessageId(message.message_id)
    return !messageId || !canonicalMessageIds.has(messageId)
  })
  // Fallback candidates: outgoing, non-draft rows this refresh newly revealed.
  // A message we were already showing before the send cannot be its server
  // copy, and a draft — even one holding this very reply — is not a sent copy.
  const known = new Set(current.map((message) => message.id))
  const candidates = refreshed.filter(
    (message) =>
      !isLocalSendId(message.id) &&
      !known.has(message.id) &&
      message.outgoing &&
      !isDraftFolder(message.folder_id, message.account_id),
  )
  // A bubble still waiting on its Message-ID has not been handed to SMTP yet, so
  // nothing this refresh reveals can be its copy — only an earlier reply of ours
  // that happens to look alike, which must not swallow it.
  const dispatched = unresolved.filter((message) => !!message.message_id || message.send_status !== 'sending')
  const paired = pairLocalSendsWithServerCopies(dispatched, candidates)
  const optimistic = unresolved.filter((message) => !paired.has(message.id))
  if (optimistic.length === 0) return refreshed
  return [...refreshed, ...optimistic].sort((a, b) => a.date - b.date)
}

/** How far the server's Date header may sit from the moment we rendered the
 * bubble and still be the same message — enough for a slow submission plus
 * modest clock skew, short enough not to swallow a genuinely later reply. */
const SENT_COPY_MATCH_WINDOW_SECONDS = 600

/**
 * Match optimistic bubbles to the server's copies of them when the Message-ID we generated didn't come back.
 *
 * Proton Bridge replaces that id with one of its own (`@protonmail.internalid`), so identity has to
 * come from the envelope: same account and sender, same subject, same recipients, and a send time
 * close to when we rendered the bubble.
 *
 * Two replies into one thread share every one of those fields, so pairing is decided globally rather
 * than by first match: every plausible pair is ranked by whether the content matches and then by how
 * far apart the two times are, and pairs are taken best-first.
 *
 * That keeps a copy arriving out of order from claiming the wrong bubble — which would hide one reply
 * and show the other twice — while still settling on time alone when a server reflows the body it
 * stored and no content match exists.
 *
 * Returns the ids of the bubbles that found a copy.
 */
function pairLocalSendsWithServerCopies(locals: Message[], candidates: Message[]): Set<string> {
  const pairs: { localId: string; candidateId: string; contentMismatch: number; skew: number }[] = []
  for (const local of locals) {
    for (const candidate of candidates) {
      if (!isPlausibleSentCopy(local, candidate)) continue
      pairs.push({
        localId: local.id,
        candidateId: candidate.id,
        contentMismatch: contentSignature(local) === contentSignature(candidate) ? 0 : 1,
        skew: Math.abs(candidate.date - local.date),
      })
    }
  }
  pairs.sort((a, b) => a.contentMismatch - b.contentMismatch || a.skew - b.skew)

  const pairedLocals = new Set<string>()
  const claimed = new Set<string>()
  for (const pair of pairs) {
    if (pairedLocals.has(pair.localId) || claimed.has(pair.candidateId)) continue
    pairedLocals.add(pair.localId)
    claimed.add(pair.candidateId)
  }
  return pairedLocals
}

/** The envelope test every pair must clear before ranking. */
function isPlausibleSentCopy(local: Message, candidate: Message): boolean {
  if (candidate.account_id !== local.account_id) return false
  if (bareAddr(candidate.from_addr ?? '') !== bareAddr(local.from_addr ?? '')) return false
  if ((candidate.subject ?? '').trim() !== (local.subject ?? '').trim()) return false
  if (recipientKey(candidate) !== recipientKey(local)) return false
  return Math.abs(candidate.date - local.date) <= SENT_COPY_MATCH_WINDOW_SECONDS
}

/** What distinguishes two replies that share an envelope: what they say and
 * what they carry. Whitespace-insensitive, since a server may rewrap the body
 * it stored — a mismatch demotes a pair rather than rejecting it. */
function contentSignature(message: Message): string {
  const body = (message.body ?? '').replace(/\s+/g, ' ').trim().toLowerCase()
  const files = (message.attachments ?? [])
    .map((attachment) => attachment.filename.trim().toLowerCase())
    .sort()
    .join('|')
  return `${files}\u0000${body}`
}

/** Order-independent set of the bare To/Cc addresses, for envelope comparison. */
function recipientKey(message: Message): string {
  return [...splitAddressList(message.to), ...splitAddressList(message.cc)]
    .map(bareAddr)
    .filter(Boolean)
    .sort()
    .join(',')
}

// The sidecar accepts RFC Message-IDs both with and without their header angle
// brackets. Cached messages retain the spelling returned by the mail server,
// so every frontend identity comparison uses this same equivalence.
export function normalizeMessageId(value: string | undefined): string {
  return (value ?? '').trim().replace(/^<|>$/g, '').toLowerCase()
}
