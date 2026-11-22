// Displays attachments belonging to the inline reply draft. Changes apply to that
// draft while the conversation remains open.
// Imported by QuickReplyComposer.tsx.

import { useState } from 'react'
import { compose$ } from '../../states/composeState'
import type { ComposerAttachment } from '../../types'
import { AttachmentImagePreview } from '../composer/AttachmentImagePreview'
import { PendingAttachmentList } from '../composer/PendingAttachmentList'

// Preview deck of the quick-reply's pending attachments. Renders nothing empty.
export function QuickReplyAttachments({ attachments }: { attachments: ComposerAttachment[] }) {
  const [previewAttachment, setPreviewAttachment] = useState<ComposerAttachment | null>(null)
  if (attachments.length === 0) return null

  return (
    <>
      <div className="flex flex-wrap gap-2 mb-1 p-1 bg-black/[0.02] dark:bg-white/[0.02] rounded-xl border border-border/10 max-h-40 overflow-y-auto select-none">
        <PendingAttachmentList
          attachments={attachments}
          onRemove={(id) => compose$.composerAttachments.set(attachments.filter((att) => att.id !== id))}
          onPreviewImage={setPreviewAttachment}
        />
      </div>
      {previewAttachment && (
        <AttachmentImagePreview attachment={previewAttachment} onClose={() => setPreviewAttachment(null)} />
      )}
    </>
  )
}
