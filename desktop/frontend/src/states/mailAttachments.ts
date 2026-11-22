// Performs desktop reader actions on message attachments and remote images.
// Opening, downloading and copying use native host commands.
// Concurrent requests for the same remote image can share the pending result.

import { attachmentSaveFilename } from '../lib/attachmentFilename'
import type { Message } from '../types'
import { invoke } from '../lib/bridge'
import { t } from '../lib/i18n'
import { showToast } from './ui'
import { isLocalSendId } from './pendingSends'

// Click-to-open an attachment: the bridge hands it to the OS default
// application. Types off its allowlist (executables, macro-capable documents,
// archives — anything a default handler could turn into code execution) come
// back `opened: false` and fall through to the save dialog, which stays the
// behaviour for everything we won't open.
export async function openAttachment(att: { key: string | null; filename: string }) {
  if (!att.key) return
  try {
    const response = await invoke<{ opened: boolean; path?: string }>('mail.openAttachment', {
      key: att.key,
      filename: att.filename,
    })
    if (!response?.opened) await downloadAttachment(att)
  } catch {
    showToast(t('chat.couldNotOpenAttachment', { filename: att.filename }))
  }
}

// Save a local attachment to disk via the native save dialog. The bytes already
// live in the media cache (keyed); the bridge copies them to the chosen path.
const remoteImageRequests = new Map<string, Promise<{ key: string; mime: string }>>()

async function cacheRemoteImage(url: string) {
  const pending = remoteImageRequests.get(url)
  if (pending) return pending
  showToast(t('common.loading'))
  const request = invoke<{ key: string; mime: string }>('mail.cacheRemoteImage', { url })
  remoteImageRequests.set(url, request)
  try {
    return await request
  } finally {
    remoteImageRequests.delete(url)
  }
}

export async function downloadAttachment(att: {
  key: string | null
  url?: string | null
  filename: string
  mime?: string
}) {
  if (!att.key && !att.url) return
  let filename = att.filename
  try {
    const cached = att.key ? att : await cacheRemoteImage(att.url!)
    filename = attachmentSaveFilename(att.filename, cached.key!, cached.mime)
    const response = await invoke<{ saved: boolean; path?: string }>('mail.saveAttachment', {
      key: cached.key,
      filename,
    })
    if (response?.saved) showToast(t('chat.savedAttachment', { filename }))
  } catch {
    showToast(t('chat.couldNotSaveAttachment', { filename }))
  }
}

// Save one message as a .eml file (its original RFC822 bytes) via the native
// save dialog. Unlike attachments, the raw bytes aren't in the media cache, so
// the bridge refetches them over IMAP — this fails offline, and a message still
// being sent has no server-side copy to fetch.
export async function saveMessageAsEml(message: Message) {
  if (!message?.id || isLocalSendId(message.id)) return
  try {
    const response = await invoke<{ saved: boolean; path?: string }>('mail.saveEml', {
      thread_id: message.thread_id,
      message_ids: [message.id],
      folder: message.folder_id,
      subject: message.subject,
    })
    if (response?.saved) showToast(t('chat.messageSaved'))
  } catch {
    showToast(t('chat.couldNotSaveMessage'))
  }
}

// Copy a keyed image onto the system clipboard. The webview's native "Copy
// Image" is inert in the Wails webview, so the bridge shells out to the same
// clipboard helpers used for pasting.
export async function copyAttachmentImage(att: { key: string | null; url?: string }) {
  if (!att.key && !att.url) return
  try {
    const cached = att.key ? att : await cacheRemoteImage(att.url!)
    await invoke('mail.copyImage', { key: cached.key })
    showToast(t('chat.imageCopied'))
  } catch {
    showToast(t('chat.couldNotCopyImage'))
  }
}
