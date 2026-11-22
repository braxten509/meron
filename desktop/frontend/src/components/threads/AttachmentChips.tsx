// Gmail-style attachment chips under a thread row:
// the first files by name, then a count of the rest.
// Display only — the row itself is the click target.

import type { Message } from '../../types'
import { fileIconFor } from '../chat/messageHelpers'

/** How many chips a row names before collapsing the rest into "+N". */
const MAX_CHIPS = 2

export function AttachmentChips({ files }: { files: NonNullable<Message['files']> }) {
  const shown = files.slice(0, MAX_CHIPS)
  const rest = files.length - shown.length
  return (
    <div className="flex min-w-0 items-center gap-1.5">
      {shown.map((file, index) => {
        const FileIcon = fileIconFor(file.filename, file.mime)
        return (
          <span
            key={index}
            title={file.filename}
            className="flex min-w-0 max-w-[45%] items-center gap-1 rounded-full border border-border px-2 py-0.5 text-[0.6875rem] text-primary/85"
          >
            <FileIcon size={12} className="shrink-0 text-accent" />
            <span className="truncate">{file.filename}</span>
          </span>
        )
      })}
      {rest > 0 && <span className="shrink-0 text-[0.6875rem] text-secondary/80">+{rest}</span>}
    </div>
  )
}
