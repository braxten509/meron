// Renders an expanded board column and connects it to dragging, loading and card
// selection. Column identity ties mutations to the correct query.
// Imported by KanbanColumnMinimized.tsx and KanbanView.tsx.

import { useCallback, useRef } from 'react'
import type { CSSProperties } from 'react'
import { useSortable } from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import { kanbanBoardColumnKey, type KanbanColumn } from '../../states/kanban'
import { type ThreadContextMenuController } from '../threads/ThreadContextMenu'
import { KanbanColumnContent } from './KanbanColumnContent'
export { type ColumnWrapper } from './KanbanColumnContent'

// A board column: both a thread drop target and a sortable item so the header
// grip can drag it to a new slot.
export function SortableColumn({
  boardId,
  column,
  dropRejection,
  onMoveThread,
  onSearchColumn,
  threadMenu,
}: {
  boardId: string
  column: KanbanColumn
  dropRejection?: string
  onMoveThread: (threadId: string, source: KanbanColumn, target: KanbanColumn) => void
  onSearchColumn: (column: KanbanColumn) => void
  threadMenu: ThreadContextMenuController
}) {
  const key = kanbanBoardColumnKey(boardId, column)
  const { setNodeRef, setActivatorNodeRef, attributes, listeners, transform, transition, isDragging, isOver, active } =
    useSortable({
      id: key,
      data: { type: 'column', column },
    })
  const nodeRef = useRef<HTMLElement | null>(null)
  const setColumnNodeRef = useCallback(
    (node: HTMLElement | null) => {
      nodeRef.current = node
      setNodeRef(node)
    },
    [setNodeRef],
  )
  const scrollColumnIntoView = useCallback(() => {
    nodeRef.current?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  }, [])
  const style: CSSProperties = {
    transform: CSS.Translate.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : undefined,
    zIndex: isDragging ? 30 : undefined,
  }
  return (
    <KanbanColumnContent
      boardId={boardId}
      column={column}
      wrapper={{
        setNodeRef: setColumnNodeRef,
        setHandleRef: setActivatorNodeRef,
        scrollIntoView: scrollColumnIntoView,
        // A column passing over during a reorder is not a drop into this one.
        isOver: isOver && active?.data.current?.type === 'thread',
        isDragging,
        dropRejection,
        style,
        dragHandle: { ...attributes, ...listeners },
      }}
      onMoveThread={onMoveThread}
      onSearchColumn={onSearchColumn}
      threadMenu={threadMenu}
    />
  )
}
