import { useEffect, useRef, useState } from 'react'
import { useValue } from '@legendapp/state/react'
import { ChevronRight, Folder, FolderOutput, FolderSymlink, Plus } from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import {
  isMeronArchive,
  meronFolders$,
  moveOutOfMeronFolder,
  moveToMeronFolder,
  promptCreateMeronFolder,
} from '../../states/meronFolders'
import { FloatingContextMenu } from '../menu/FloatingContextMenu'
import { MenuItem } from '../menu/MenuItem'
import { flyoutClass, useFlyoutPosition } from './ThreadMoveCopyItems'

const FLYOUT_WIDTH = 220

/**
 * "Move to Meron folder ▸" and "Move out of Meron folder", shared by the
 * thread list's right-click menu, the open conversation's actions menu and the
 * bulk bar's menu. Meron folders exist only on this device; the flyout lists
 * them A–Z (minus the one the conversation is in) and ends with "New Meron
 * folder", which makes one and moves there. Callers decide whether the rows
 * apply at all (not for sender rows, feeds or local folders).
 */
export function MeronFolderItems({
  threadIds,
  currentFolderId,
  close,
  onDone,
  itemClassName,
  iconSize = 13,
  dataAttribute = 'data-thread-context-menu',
}: {
  threadIds: string[]
  /** The Meron folder the conversation sits in; set only for one thread. */
  currentFolderId?: string | null
  close: () => void
  onDone?: () => void
  itemClassName?: string
  iconSize?: number
  /** The marker the hosting menu's "click outside" check counts as inside. */
  dataAttribute?: string
}) {
  const { t } = useTranslation()
  const folders = useValue(meronFolders$.list)
  const [open, setOpen] = useState(false)
  const anchorRef = useRef<HTMLDivElement | null>(null)
  const position = useFlyoutPosition(open, anchorRef, FLYOUT_WIDTH)
  const key = threadIds.join('\n')

  useEffect(() => setOpen(false), [key])

  if (!threadIds.length) return null
  // The built-in Archive is filled by archiving, never by this menu.
  const targets = folders.filter((folder) => folder.id !== currentFolderId && !isMeronArchive(folder))

  const moveTo = (folderId: string) => {
    close()
    void moveToMeronFolder(threadIds, folderId).then((moved) => {
      if (moved) onDone?.()
    })
  }

  return (
    <>
      <div ref={anchorRef} onMouseEnter={() => setOpen(true)} onMouseLeave={() => setOpen(false)}>
        <MenuItem
          icon={<FolderSymlink size={iconSize} className="shrink-0 text-secondary" />}
          label={t('meronFolders.moveTo')}
          submenuOpen={open}
          trailing={<ChevronRight size={13} className="text-secondary" />}
          className={itemClassName}
          onClick={() => setOpen(true)}
        />
        {open && position && (
          <FloatingContextMenu
            x={position.x}
            y={position.y}
            className={`${flyoutClass} min-w-[220px]`}
            dataAttribute={dataAttribute}
          >
            {targets.length === 0 && (
              <div className="px-3 py-2 text-xs font-semibold text-secondary">{t('meronFolders.none')}</div>
            )}
            {targets.map((folder) => (
              <MenuItem
                key={folder.id}
                icon={<Folder size={13} className="shrink-0 text-secondary" />}
                label={
                  <span className="min-w-0 max-w-[260px] truncate" title={folder.name}>
                    {folder.name}
                  </span>
                }
                onClick={() => moveTo(folder.id)}
              />
            ))}
            <div className="my-1 border-t border-border" />
            <MenuItem
              icon={<Plus size={13} className="shrink-0 text-secondary" />}
              label={t('meronFolders.new')}
              onClick={() => {
                close()
                void promptCreateMeronFolder().then((folder) => {
                  if (!folder) return
                  return moveToMeronFolder(threadIds, folder.id).then((moved) => {
                    if (moved) onDone?.()
                  })
                })
              }}
            />
          </FloatingContextMenu>
        )}
      </div>
      {currentFolderId && (
        <MenuItem
          icon={<FolderOutput size={iconSize} className="shrink-0 text-secondary" />}
          // Out of Archive is back to the Inbox (un-archived on the server).
          label={isMeronArchive(currentFolderId) ? t('meronFolders.moveToInbox') : t('meronFolders.moveOut')}
          className={itemClassName}
          onClick={() => {
            close()
            void moveOutOfMeronFolder(threadIds).then((moved) => {
              if (moved) onDone?.()
            })
          }}
        />
      )}
    </>
  )
}
