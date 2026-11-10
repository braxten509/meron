// Sender rows (group by sender) are ordinary thread ids whose key is
// `sender:<lowercase address>`: `account#folder#t.<base64url(key)>`, the same
// shape the bridge mints for every thread card (formatImapThreadID). These
// helpers read and build that key on the client side.

const THREAD_KEY_MARK = '#t.'
const SENDER_KEY_PREFIX = 'sender:'

function encodeBase64Url(text: string): string {
  const bytes = new TextEncoder().encode(text)
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function decodeBase64Url(encoded: string): string | null {
  try {
    const padded = encoded.replace(/-/g, '+').replace(/_/g, '/')
    const binary = atob(padded + '='.repeat((4 - (padded.length % 4)) % 4))
    const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0))
    return new TextDecoder().decode(bytes)
  } catch {
    return null
  }
}

/** The encoded thread key of an IMAP thread id, or null for other ids (a bare
 * UID, a feed). Base64url has no `#` or `.`, so the last mark is the split. */
function threadKeyOf(threadId: string): string | null {
  const at = threadId.lastIndexOf(THREAD_KEY_MARK)
  if (at < 0) return null
  return decodeBase64Url(threadId.slice(at + THREAD_KEY_MARK.length))
}

/** The conversation a thread id belongs to, as `account\n<root key>`: subject
 * branches of one root (`<root>#<subject>`, as the core's
 * `split_branch_compound_key`) share it. The id itself for other ids. */
export function conversationRootOf(threadId: string): string {
  const key = threadKeyOf(threadId)
  if (key === null) return threadId
  const account = threadId.slice(0, Math.max(0, threadId.indexOf('#')))
  const branches = !key.startsWith('uid:') && !key.startsWith('gmthrid:') && !key.startsWith(SENDER_KEY_PREFIX)
  const root = branches && key.includes('#') ? key.slice(0, key.indexOf('#')) : key
  return `${account}\n${root.replace(/%23/g, '#').replace(/%25/g, '%').toLowerCase()}`
}

/** Whether a thread id names a sender row's chat rather than a conversation. */
export function isSenderThreadId(threadId: string): boolean {
  return threadKeyOf(threadId)?.startsWith(SENDER_KEY_PREFIX) ?? false
}

/** The sender chat id for `address` in the same account and folder as
 * `threadId` (an IMAP thread id); null when `threadId` is not one. */
export function senderThreadIdLike(threadId: string, address: string): string | null {
  const at = threadId.lastIndexOf(THREAD_KEY_MARK)
  const addr = address.trim().toLowerCase()
  if (at < 0 || !addr) return null
  return `${threadId.slice(0, at)}${THREAD_KEY_MARK}${encodeBase64Url(SENDER_KEY_PREFIX + addr)}`
}

/** The person a sender chat id stands for (lowercase address), or null when
 * the id is an ordinary thread. */
export function senderAddressOf(threadId: string): string | null {
  const key = threadKeyOf(threadId)
  if (!key?.startsWith(SENDER_KEY_PREFIX)) return null
  return key.slice(SENDER_KEY_PREFIX.length) || null
}

/** Name and address shown at the top of a sender chat; null for an ordinary
 * thread. The list row knows the name; a chat opened without its row (a
 * notification, a tab) takes it from the person's own mail in the chat, and
 * otherwise shows the address alone. */
export function senderChatIdentity(
  thread: { thread_id: string; group?: string; from_name: string; from_addr: string },
  messages: ReadonlyArray<{ thread_id: string; from_name: string; from_addr: string }>,
): { name: string; address: string } | null {
  const address = senderAddressOf(thread.thread_id)
  if (!address) return null
  if (thread.group === 'sender') {
    return { name: thread.from_name.trim() || address, address: thread.from_addr.trim() || address }
  }
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    if (message.thread_id !== thread.thread_id) continue
    if (message.from_addr.trim().toLowerCase() === address && message.from_name.trim()) {
      return { name: message.from_name.trim(), address }
    }
  }
  return { name: address, address }
}

const REPLY_PREFIX = /^\s*(?:(?:re|fwd?|aw|sv|wg|vs)\s*(?:\[\d+\])?\s*:\s*)+/i

/** A subject with its reply/forward prefixes dropped, for telling whether two
 * emails in a sender chat are about the same thing. */
export function subjectTopic(subject: string | undefined): string {
  return (subject ?? '').replace(REPLY_PREFIX, '').trim().toLowerCase()
}
