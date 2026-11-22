// Edits the saved search displayed in a board column.
// The query editor coordinates focus and commits changes to the board.
// The column supplies the current query and editing callbacks.

import { useEffect, useRef, useState } from 'react'
import { useTranslation } from '../../lib/i18n'
import { Search } from 'lucide-react'
import { clsx } from '../../lib/classNames'
import { SearchOptions } from '../search/SearchOptions'

// A saved search column's query beside its folder name, edited in place. It sits
// inside the header's drag handle, so presses and keys in it stay its own: Enter
// or Space would otherwise pick the column up.
export function SavedSearchQuery({
  query,
  editing,
  onStartEdit,
  onCommit,
  onCancel,
}: {
  query: string
  editing: boolean
  onStartEdit: () => void
  onCommit: (query: string) => void
  onCancel: () => void
}) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState(query)
  // Cancelled, or committed from the options form: leaving the editor then commits nothing.
  const settled = useRef(false)
  const editorRef = useRef<HTMLSpanElement | null>(null)
  const inputRef = useRef<HTMLInputElement | null>(null)
  useEffect(() => {
    if (!editing) return
    setDraft(query)
    settled.current = false
  }, [editing, query])
  const chipClass =
    'flex min-w-0 items-center gap-1 rounded-md bg-accent/10 px-1.5 py-0.5 text-[0.625rem] font-semibold text-accent'
  const keepEvent = (event: { stopPropagation: () => void }) => event.stopPropagation()

  if (editing) {
    return (
      // Not positioned, so the options form lays out against its row, as wide as
      // the column; it still sits inside the editor, so focus moving into it
      // doesn't count as leaving.
      <span
        ref={editorRef}
        className={clsx(chipClass, 'flex-1')}
        onPointerDown={keepEvent}
        onClick={keepEvent}
        onKeyDown={keepEvent}
        onBlur={(event) => {
          if (settled.current || editorRef.current?.contains(event.relatedTarget as Node | null)) return
          onCommit(draft)
        }}
        title=""
      >
        <Search size={10} className="shrink-0" />
        <input
          ref={inputRef}
          autoFocus
          value={draft}
          aria-label={t('kanban.actions.editSearch')}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            event.stopPropagation()
            // Enter and Escape belong to an IME while it composes. WebKit reports
            // the keystroke that ends a composition as keyCode 229 instead.
            if (event.nativeEvent.isComposing || event.keyCode === 229) return
            if (event.key === 'Enter') event.currentTarget.blur()
            if (event.key === 'Escape') {
              settled.current = true
              onCancel()
            }
          }}
          className="min-w-0 flex-1 border-0 bg-transparent p-0 text-[0.625rem] font-semibold text-primary outline-none"
        />
        <SearchOptions
          query={draft}
          anchorRef={editorRef}
          onSearch={(next) => {
            settled.current = true
            onCommit(next)
          }}
          onDismiss={() => inputRef.current?.focus()}
          submitLabel={t('buttons.save')}
        />
      </span>
    )
  }
  return (
    <button
      type="button"
      className={clsx(chipClass, 'cursor-pointer hover:bg-accent/15')}
      title={query}
      aria-label={t('kanban.actions.editSearch')}
      onPointerDown={keepEvent}
      onClick={(event) => {
        event.stopPropagation()
        onStartEdit()
      }}
    >
      <Search size={10} className="shrink-0" />
      <span className="truncate">{query}</span>
    </button>
  )
}
