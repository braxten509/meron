// Edits a search within the board and resolves the selected scope. Column queries
// and account boundaries determine which results can be displayed.
// Imported by KanbanView.tsx and TitleBar.tsx.

import { useEffect, useMemo, useRef, useState } from 'react'
import { Search, X } from 'lucide-react'
import { useValue } from '@legendapp/state/react'
import { useTranslation } from '../../lib/i18n'
import {
  getKanbanColumns,
  kanban$,
  kanbanBoardColumnKey,
  kanbanColumnKey,
  saveKanbanSearchColumn,
} from '../../states/kanban'
import { loadKanbanColumn } from '../../lib/kanbanData'
import { settings$ } from '../../states/settings'
import { ui$ } from '../../states/ui'
import { IconButton } from '../button/IconButton'
import { SearchBox } from '../search/SearchBox'
import { ColumnAvatar, useColumnLabel } from './ColumnLabel'

/**
 * A kanban board's search box, with the column it is scoped to. It sits in the title bar
 * when Meron draws one, always open there; in the board's header otherwise,
 * collapsed to an icon until it is wanted.
 */
export function KanbanSearch({ boardId, compact = false }: { boardId: string; compact?: boolean }) {
  const { t } = useTranslation()
  const boards = useValue(settings$.kanbanBoards)
  const searchQuery = useValue(kanban$.searchQuery)
  const searchScope = useValue(kanban$.searchScope)
  const globalSearchFocus = useValue(ui$.globalSearchFocus)
  const visibleColumns = useMemo(() => getKanbanColumns(boardId), [boards, boardId])
  const scopedColumn = visibleColumns.find((column) => kanbanColumnKey(column) === searchScope)
  const columnLabel = useColumnLabel()
  // The header's bar is collapsed to an icon by default; it expands on click (or
  // the search hotkey) and folds back once it's empty and loses focus. Start open
  // if a query is already active so a persisted search stays visible.
  const [expanded, setExpanded] = useState(() => !!kanban$.searchQuery.peek().trim())
  const open = compact || expanded
  const inputRef = useRef<HTMLInputElement | null>(null)
  const barRef = useRef<HTMLDivElement | null>(null)

  // Bumped by the search hotkey, the palette and a column's "search this column".
  useEffect(() => {
    if (globalSearchFocus === 0) return
    setExpanded(true)
    requestAnimationFrame(() => {
      inputRef.current?.focus()
      inputRef.current?.select()
    })
  }, [globalSearchFocus])

  // Focus the input whenever the header's bar expands so it's immediately typeable.
  useEffect(() => {
    if (expanded && !compact) inputRef.current?.focus()
  }, [expanded, compact])

  // Collapse the header's bar when the user clicks away, but only if it's empty —
  // an active query keeps the bar (and its results) visible.
  useEffect(() => {
    if (!expanded || compact) return
    const onPointerDown = (event: MouseEvent) => {
      if (barRef.current?.contains(event.target as Node)) return
      if (kanban$.searchQuery.peek().trim()) return
      setExpanded(false)
    }
    document.addEventListener('mousedown', onPointerDown)
    return () => document.removeEventListener('mousedown', onPointerDown)
  }, [expanded, compact])

  if (!open) {
    return <IconButton icon={Search} label={t('kanban.searchBoardAction')} onClick={() => setExpanded(true)} />
  }

  const setScope = (scope: string) => {
    kanban$.searchScope.set(scope)
    expandSearchScope(boardId, scope)
  }
  const scopeLabel = scopedColumn ? columnLabel(scopedColumn) : null

  return (
    <SearchBox
      value={searchQuery}
      onChange={(query) => kanban$.searchQuery.set(query)}
      placeholder={t('kanban.searchBoard')}
      compact={compact}
      className={compact ? 'w-full' : 'basis-72 shrink'}
      inputRef={inputRef}
      boxRef={barRef}
      // A search narrowed to one column says which; clearing it searches them all.
      chip={
        scopedColumn &&
        scopeLabel && (
          <span
            title={scopeLabel.account ? `${scopeLabel.folder} · ${scopeLabel.account}` : scopeLabel.folder}
            className="flex max-w-[40%] shrink-0 items-center gap-1 rounded-md bg-accent/10 py-0.5 pr-0.5 pl-1 text-[0.6875rem] font-semibold text-accent"
          >
            <ColumnAvatar column={scopedColumn} size={14} />
            <span className="truncate">{scopeLabel.folder}</span>
            <button
              type="button"
              onClick={() => {
                setScope('all')
                inputRef.current?.focus()
              }}
              aria-label={t('kanban.searchScope.allColumns')}
              title={t('kanban.searchScope.allColumns')}
              className="flex cursor-pointer rounded p-0.5 hover:bg-accent/15"
            >
              <X size={11} />
            </button>
          </span>
        )
      }
      // Backspace in an empty box takes the scope off, as with any token before a caret.
      onKeyDown={(event) => {
        if (event.key === 'Backspace' && !searchQuery && scopedColumn) setScope('all')
      }}
      onEscapeEmpty={compact ? undefined : () => setExpanded(false)}
      options={{
        onSearch: (query, scope) => {
          if (scope !== undefined && scope !== searchScope) setScope(scope)
          kanban$.searchQuery.set(query)
          inputRef.current?.focus()
        },
        board: {
          scope: searchScope,
          columns: visibleColumns,
          onSave: (query, column) => {
            const saved = saveKanbanSearchColumn(boardId, column, query)
            if (saved) void loadKanbanColumn(saved, true)
          },
        },
      }}
    />
  )
}

// Matches are drawn in the column body, so scoping a search to a collapsed
// column would hide the very results it just loaded behind the strip.
export function expandSearchScope(boardId: string, scope: string) {
  const column = getKanbanColumns(boardId).find((item) => kanbanColumnKey(item) === scope)
  if (!column) return
  settings$.kanbanMinimizedColumns[kanbanBoardColumnKey(boardId, column)].set(false)
}
