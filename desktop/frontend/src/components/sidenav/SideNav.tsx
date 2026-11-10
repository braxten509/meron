import { useEffect, useState } from 'react'
import type { DragEvent, KeyboardEvent, MouseEvent } from 'react'
import {
  Bell,
  BellOff,
  CalendarClock,
  CalendarDays,
  CheckCheck,
  Folder,
  Mail,
  SquareCheckBig,
  MoreHorizontal,
  EyeOff,
  Pencil,
  Plus,
  SquarePen,
  Trash2,
  Users,
} from 'lucide-react'
import { useValue } from '@legendapp/state/react'
import { useTranslation } from '../../lib/i18n'
import { formatShortcut, isMac, RAIL_SHORTCUT_IDS } from '../../lib/shortcuts'
import { DndContext, closestCenter, KeyboardSensor, PointerSensor, useSensor, useSensors } from '@dnd-kit/core'
import type { DragEndEvent } from '@dnd-kit/core'
import { SortableContext, sortableKeyboardCoordinates, verticalListSortingStrategy } from '@dnd-kit/sortable'
import { restrictToVerticalAxis } from '@dnd-kit/modifiers'
import { accounts$, isSendableAccount, reorderAccountIds } from '../../states/accounts'
import { openComposeTab } from '../../states/compose'
import { moveFeed, RSS_FEED_DRAG_TYPE } from '../../states/feeds'
import { kanban$, openMailAccount, reorderKanbanBoards, selectKanbanBoard } from '../../states/kanban'
import { toggleContactsPanel } from '../../states/contacts'
import { mail$ } from '../../states/mail'
import { inboxUnread } from '../../states/mailFolders'
import { settings$, setCalendarSideNavVisible, setUnifiedInboxSideNavVisible } from '../../states/settings'
import { calendarPanelShowing, openCalendar, toggleCalendarPanel } from '../../states/calendar'
import { useNarrowWindow } from '../../lib/useNarrowWindow'
import { toggleTasksPanel } from '../../states/tasks'
import { ui$ } from '../../states/ui'
import { QuickSettingsMenu } from './QuickSettingsMenu'
import { NavRow, NavSectionHeading, SortableBoard, SortableAccount, railIndicator } from './SortableRailItems'
import { RailContextMenu, RailMenuItem } from './RailContextMenu'
import { AccountContextMenu } from './AccountContextMenu'
import { BoardContextMenu } from './BoardContextMenu'
import { BoardDialog, type BoardDialogState } from './BoardDialog'
import { useTitleBar } from '../titlebar/TitleBar'
import type { Account } from '../../types'
import { isMeronView } from '../../lib/unifiedFolders'
import {
  confirmDeleteMeronFolder,
  isMeronArchive,
  markMeronFolderRead,
  meronFolders$,
  openMeronFolder,
  promptCreateMeronFolder,
  promptRenameMeronFolder,
  setMeronFolderNotify,
  type MeronFolder,
} from '../../states/meronFolders'

// The compose button's divider lines up with the 48px header borders and so
// sits 6px under the button; the others sit 9px from their neighbours. Each
// spans the side navigation inside its 8px side padding, like the rows.
const railDivider = 'mx-2 h-px shrink-0 self-stretch bg-sidenav-ink/10'

// A Meron folder row is a real button, so Tab reaches it and Enter or Space
// opens it; the ring shows for keyboard focus only.
const meronRowButton =
  'block w-full cursor-pointer rounded-lg text-left outline-none focus-visible:ring-2 focus-visible:ring-accent/40'

// Utility buttons along the bottom: one square size, one radius, so their
// icons share a baseline with each other.
const utilityButton = (pressed: boolean) =>
  `flex h-9 w-9 items-center justify-center rounded-lg transition-colors cursor-pointer ${
    pressed
      ? 'bg-sidenav-ink/15 text-sidenav-ink'
      : 'text-sidenav-ink/60 hover:bg-sidenav-ink/[0.07] hover:text-sidenav-ink'
  }`

export function SideNav() {
  const { t } = useTranslation()
  const accounts = useValue(accounts$)
  useValue(settings$.shortcutOverrides)
  const tasksEnabled = useValue(settings$.tasksEnabled)
  const tasksPanelOpen = useValue(ui$.tasksPanelOpen)
  const calendarEnabled = useValue(settings$.calendarEnabled)
  const calendarOpen = useValue(ui$.calendarOpen)
  const contactsOpen = useValue(ui$.contactsOpen)
  const showCalendar = useValue(settings$.showCalendarInSideNav) && calendarEnabled
  const calendarPanelOpen = useValue(ui$.calendarPanelOpen)
  const narrow = useNarrowWindow()
  // The utility button toggles the panel, or the full view on a narrow window
  // where the panel cannot show.
  const calendarButtonPressed = narrow ? calendarOpen : calendarPanelShowing(calendarPanelOpen, calendarOpen, narrow)
  const boards = useValue(settings$.kanbanBoards)
  const hiddenSideNavAccounts = useValue(settings$.hiddenSideNavAccounts)
  const showUnifiedInbox = useValue(settings$.showUnifiedInboxInSideNav)
  const showUnreadBadge = useValue(settings$.showUnreadAccountBadge)
  const foldersByAccount = useValue(mail$.foldersByAccount)
  const activeBoardId = useValue(kanban$.activeBoardId)
  const selectedAccount = useValue(ui$.selectedAccount)
  const hasSendableAccount = accounts.some(isSendableAccount)
  // Tasks and the more menu live in Meron's own title bar when there is one.
  const titleBar = useTitleBar()
  const utilities = !titleBar
  // Right-click context menu anchored at the cursor for one account.
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null)
  const menuAccount = menu ? accounts.find((acc) => acc.id === menu.id) : null
  const [boardMenu, setBoardMenu] = useState<{ id: string; x: number; y: number } | null>(null)
  const menuBoard = boardMenu ? boards.find((board) => board.id === boardMenu.id) : null
  const [boardDialog, setBoardDialog] = useState<BoardDialogState | null>(null)
  const [unifiedMenu, setUnifiedMenu] = useState<{ x: number; y: number } | null>(null)
  const [calendarMenu, setCalendarMenu] = useState<{ x: number; y: number } | null>(null)
  // Bottom "more" menu holding the view switcher and theme settings.
  const [moreMenu, setMoreMenu] = useState<{ x: number; y: number } | null>(null)
  // Meron folders (this device only), A–Z from the core, above the accounts.
  const meronFolders = useValue(meronFolders$.list)
  // `keyboard` marks a menu opened with the Menu key or Shift+F10: it takes
  // focus, and gives it back to the row when it closes.
  const [meronMenu, setMeronMenu] = useState<{ folder: MeronFolder; x: number; y: number; keyboard: boolean } | null>(
    null,
  )
  const selectedFolder = useValue(ui$.selectedFolder)
  const isMeronActive = (id: string) =>
    !calendarOpen && !activeBoardId && isMeronView(selectedAccount) && selectedFolder === id

  useEffect(() => {
    if (!meronMenu?.keyboard) return
    const frame = requestAnimationFrame(() =>
      document.querySelector<HTMLElement>('[data-meron-folder-menu] button:not(:disabled)')?.focus(),
    )
    return () => cancelAnimationFrame(frame)
  }, [meronMenu])

  const closeMeronMenu = (restoreFocus = true) => {
    const menuState = meronMenu
    setMeronMenu(null)
    if (restoreFocus && menuState?.keyboard) {
      document.querySelector<HTMLElement>(`[data-meron-folder-row="${CSS.escape(menuState.folder.id)}"]`)?.focus()
    }
  }

  // A Meron folder row's Rename/Delete menu: right-click at the pointer; the
  // Menu key or Shift+F10 under the row (a keyboard contextmenu event has no
  // pointer position).
  const openMeronMenuAt = (folder: MeronFolder, element: HTMLElement, keyboard: boolean, x = 0, y = 0) => {
    if (keyboard || (x === 0 && y === 0)) {
      const rect = element.getBoundingClientRect()
      setMeronMenu({ folder, x: rect.left + 16, y: rect.bottom, keyboard: true })
    } else {
      setMeronMenu({ folder, x, y, keyboard: false })
    }
  }
  const meronRowContextMenu = (folder: MeronFolder) => (event: MouseEvent<HTMLButtonElement>) => {
    event.preventDefault()
    event.stopPropagation()
    openMeronMenuAt(folder, event.currentTarget, false, event.clientX, event.clientY)
  }
  const meronRowKeyDown = (folder: MeronFolder) => (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10')) return
    event.preventDefault()
    event.stopPropagation()
    openMeronMenuAt(folder, event.currentTarget, true)
  }

  // Starred is a folder of the unified view, reachable from the column's folder
  // switcher, so the rail button covers it too.
  const isUnifiedActive = !calendarOpen && !activeBoardId && selectedAccount === 'unified'
  const unifiedUnread = showUnreadBadge
    ? accounts.reduce(
        (sum, account) =>
          account.included_in_unified !== false ? sum + inboxUnread(foldersByAccount[account.id]) : sum,
        0,
      )
    : 0
  const hiddenSideNavAccountIds = new Set(hiddenSideNavAccounts)
  const sideNavAccounts = accounts.filter((account) => !hiddenSideNavAccountIds.has(account.id))
  const hasBoards = boards.length > 0
  const hasAccounts = sideNavAccounts.length > 0
  const railShortcut = (index: number) => {
    const id = RAIL_SHORTCUT_IDS[index]
    return id ? formatShortcut(id).join(isMac ? '' : '+') : undefined
  }

  const isRssAccount = (account: { provider: string; auth_type: string }) =>
    account.provider === 'rss' || account.auth_type === 'rss'

  function parseFeedDrag(event: DragEvent): { threadId: string; accountId?: string } | null {
    if (!Array.from(event.dataTransfer.types).includes(RSS_FEED_DRAG_TYPE)) return null
    try {
      const raw = event.dataTransfer.getData(RSS_FEED_DRAG_TYPE)
      const parsed = JSON.parse(raw) as { threadId?: string; accountId?: string }
      if (!parsed.threadId) return null
      return { threadId: parsed.threadId, accountId: parsed.accountId }
    } catch {
      return null
    }
  }

  function handleFeedDragOver(event: DragEvent<HTMLDivElement>, account: Account) {
    if (!isRssAccount(account)) return
    const payload = parseFeedDrag(event)
    if (!payload || payload.accountId === account.id) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'move'
  }

  function handleFeedDrop(event: DragEvent<HTMLDivElement>, account: Account) {
    if (!isRssAccount(account)) return
    const payload = parseFeedDrag(event)
    if (!payload || payload.accountId === account.id) return
    event.preventDefault()
    event.stopPropagation()
    void moveFeed(payload.threadId, account.id)
  }

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  )

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event
    if (over && active.id !== over.id) {
      const visible = [...sideNavAccounts]
      const oldIndex = visible.findIndex((account) => account.id === active.id)
      const newIndex = visible.findIndex((account) => account.id === over.id)
      if (oldIndex === -1 || newIndex === -1) return
      const [removed] = visible.splice(oldIndex, 1)
      visible.splice(newIndex, 0, removed)
      const next = [...accounts]
      const visiblePositions = accounts.flatMap((account, index) =>
        hiddenSideNavAccountIds.has(account.id) ? [] : [index],
      )
      visiblePositions.forEach((position, index) => {
        next[position] = visible[index]
      })
      void reorderAccountIds(next.map((account) => account.id))
    }
  }

  function handleBoardDragEnd(event: DragEndEvent) {
    const { active, over } = event
    if (over && active.id !== over.id) {
      const oldIndex = boards.findIndex((board) => board.id === active.id)
      const newIndex = boards.findIndex((board) => board.id === over.id)
      reorderKanbanBoards(oldIndex, newIndex)
    }
  }

  const selectAccount = (id: string, folderId = 'inbox') => openMailAccount(id, folderId)

  return (
    <aside
      // A sidebar with names: Meron folders on top, then the accounts. Sized
      // in rem so it grows with the Text size setting like the rows inside it.
      className={`flex w-56 shrink-0 flex-col bg-sidenav px-0 pb-3 max-[768px]:hidden select-none ${titleBar ? 'pt-[calc(5px+var(--me-frame-border-width))]' : 'pt-[10px]'}`}
      onContextMenu={(event) => {
        if (event.defaultPrevented) return
        event.preventDefault()
        setMoreMenu({ x: event.clientX, y: event.clientY })
      }}
    >
      {hasSendableAccount && (
        <>
          <button
            // An action, not a destination: a round-ended button rather than a
            // row, lifting on hover. Same 36px height as the rail's button, so
            // the divider under it still meets the header borders.
            className="mx-2 flex h-9 shrink-0 items-center gap-2.5 rounded-full bg-accent px-3.5 text-[0.8125rem] font-semibold leading-5 text-accent-label transition-[background-color,color,box-shadow] duration-200 ease-out hover:bg-accent-hover hover:text-accent-hover-label hover:shadow-md cursor-pointer"
            onClick={() => openComposeTab()}
            title={`${t('composer.actions.newMessage')} (${formatShortcut('compose.new').join(isMac ? '' : '+')})`}
            aria-label={t('composer.actions.newMessage')}
          >
            <SquarePen size={16} className="shrink-0" />
            <span className="truncate">{t('composer.actions.newMessage')}</span>
          </button>
          {/* 5 + 36 + 6 aligns with the 48px header's bottom border, plus the
            content frame's border where the theme draws one, and 4px more
            under the system title bar, where the frame stands clear of the
            top edge and always has its border. */}
          <div className={`mt-[6px] ${railDivider}`} />
        </>
      )}
      <div className="flex min-h-0 w-full flex-1 flex-col overflow-y-auto no-scrollbar py-[9px]">
        {/* Meron folders: they exist only in Meron, on this device, and take
          conversations from every account, so they head the list. */}
        <NavSectionHeading first>{t('meronFolders.title')}</NavSectionHeading>
        {meronFolders.filter((folder) => !isMeronArchive(folder)).map((folder) => {
          const active = isMeronActive(folder.id)
          return (
            <div key={folder.id} className="relative w-full px-2 group">
              {railIndicator(active)}
              <button
                type="button"
                data-meron-folder-row={folder.id}
                className={meronRowButton}
                onClick={() => openMeronFolder(folder.id)}
                onContextMenu={meronRowContextMenu(folder)}
                onKeyDown={meronRowKeyDown(folder)}
                title={`${folder.name} · ${t('meronFolders.thisDeviceOnly')}`}
                aria-current={active ? 'page' : undefined}
                // Rename and Delete open from the keyboard too.
                aria-keyshortcuts="Shift+F10 ContextMenu"
              >
                <NavRow active={active} icon={<Folder size={16} />} label={folder.name} count={folder.unread} />
              </button>
            </div>
          )
        })}
        <div className="relative w-full px-2 group">
          <button
            type="button"
            className={meronRowButton}
            onClick={() =>
              void promptCreateMeronFolder().then((folder) => {
                if (folder) openMeronFolder(folder.id)
              })
            }
            title={t('meronFolders.thisDeviceOnly')}
          >
            <NavRow active={false} quiet icon={<Plus size={16} />} label={t('meronFolders.new')} />
          </button>
        </div>
        {/* The built-in Archive: always last, under a thin line, the same
          folder icon in gray. It can't be renamed or deleted, so no menu. */}
        {meronFolders
          .filter((folder) => isMeronArchive(folder))
          .map((folder) => {
            const active = isMeronActive(folder.id)
            const name = t('folders.roles.archive')
            return (
              <div key={folder.id}>
                <div className={`my-[6px] ${railDivider}`} />
                <div className="relative w-full px-2 group">
                  {railIndicator(active)}
                  <button
                    type="button"
                    data-meron-folder-row={folder.id}
                    className={meronRowButton}
                    onClick={() => openMeronFolder(folder.id)}
                    title={`${name} · ${t('meronFolders.thisDeviceOnly')}`}
                    aria-current={active ? 'page' : undefined}
                  >
                    <NavRow
                      active={active}
                      icon={<Folder size={16} className="text-sidenav-ink/45" />}
                      label={name}
                      count={folder.unread}
                    />
                  </button>
                </div>
              </div>
            )
          })}

        {(showUnifiedInbox || hasAccounts) && (
          // TODO(text): a plain "Accounts" heading has no key yet.
          <NavSectionHeading>{t('sidenav.accounts')}</NavSectionHeading>
        )}

        {/* Unified Inbox: every account's Inbox, so it leads the accounts.
          Starred is a folder of the unified view, reachable from the
          column's folder switcher, so this row covers it too. */}
        {showUnifiedInbox && (
          <div
            className="relative w-full px-2 group cursor-pointer"
            onClick={() => selectAccount('unified')}
            onContextMenu={(event) => {
              event.preventDefault()
              event.stopPropagation()
              setUnifiedMenu({ x: event.clientX, y: event.clientY })
            }}
            title={`${t('settings.sideNav.showUnifiedInbox')} (${railShortcut(0)})`}
            aria-current={isUnifiedActive ? 'page' : undefined}
          >
            {railIndicator(isUnifiedActive)}
            <NavRow
              active={isUnifiedActive}
              icon={<Mail size={16} />}
              label={t('kanban.columns.unifiedInbox')}
              count={unifiedUnread}
            />
          </div>
        )}

        {/* Accounts List */}
        {hasAccounts && (
          <div className="flex w-full flex-col">
            <DndContext
              sensors={sensors}
              collisionDetection={closestCenter}
              onDragEnd={handleDragEnd}
              modifiers={[restrictToVerticalAxis]}
            >
              <SortableContext items={sideNavAccounts.map((acc) => acc.id)} strategy={verticalListSortingStrategy}>
                {sideNavAccounts.map((account, index) => (
                  <SortableAccount
                    key={account.id}
                    account={account}
                    shortcut={railShortcut(Number(showUnifiedInbox) + boards.length + index)}
                    active={!calendarOpen && !activeBoardId && account.id === selectedAccount}
                    onSelect={() => selectAccount(account.id)}
                    onContextMenu={(e) => {
                      e.preventDefault()
                      e.stopPropagation()
                      setMenu({ id: account.id, x: e.clientX, y: e.clientY })
                    }}
                    onFeedDragOver={handleFeedDragOver}
                    onFeedDrop={handleFeedDrop}
                  />
                ))}
              </SortableContext>
            </DndContext>
          </div>
        )}

        {/* Kanban Boards. Their number shortcuts still come right after the
          unified inbox's, as on the rail. */}
        {hasBoards && (
          <>
            <NavSectionHeading>{t('settings.sections.kanbanBoards')}</NavSectionHeading>
            <div className="flex w-full flex-col">
              <DndContext
                sensors={sensors}
                collisionDetection={closestCenter}
                onDragEnd={handleBoardDragEnd}
                modifiers={[restrictToVerticalAxis]}
              >
                <SortableContext items={boards.map((board) => board.id)} strategy={verticalListSortingStrategy}>
                  {boards.map((board, index) => (
                    <SortableBoard
                      key={board.id}
                      board={board}
                      shortcut={railShortcut(Number(showUnifiedInbox) + index)}
                      active={!calendarOpen && board.id === activeBoardId}
                      onSelect={() => selectKanbanBoard(board.id)}
                      onContextMenu={(e) => {
                        e.preventDefault()
                        e.stopPropagation()
                        setBoardMenu({ id: board.id, x: e.clientX, y: e.clientY })
                      }}
                    />
                  ))}
                </SortableContext>
              </DndContext>
            </div>
          </>
        )}
      </div>

      {/* The full calendar: a view, like the unified inbox and the boards, so a
        destination in the rail, but not mail, so pinned under their scrolling
        list rather than in it — always in the same place, never scrolled away.
        It has no number shortcut; those count the items above. The panel
        toggle among the utilities (or in the title bar) has its own icon. */}
      {showCalendar && (
        <>
          <div className={`mb-[9px] ${railDivider}`} />
          <div className="relative w-full px-2 group">
            {railIndicator(calendarOpen)}
            <button
              type="button"
              className="block w-full cursor-pointer text-left"
              onClick={openCalendar}
              onContextMenu={(event) => {
                event.preventDefault()
                event.stopPropagation()
                setCalendarMenu({ x: event.clientX, y: event.clientY })
              }}
              title={t('calendar.title')}
              aria-label={t('calendar.title')}
              aria-current={calendarOpen ? 'page' : undefined}
            >
              <NavRow active={calendarOpen} icon={<CalendarDays size={16} />} label={t('calendar.title')} />
            </button>
          </div>
        </>
      )}

      {utilities && <div className={`${showCalendar ? 'my-[9px]' : 'mb-[9px]'} ${railDivider}`} />}

      {/* Utilities */}
      {utilities && (
        <div className="flex items-center gap-1 px-2">
          <button
            type="button"
            onClick={toggleContactsPanel}
            title={t('contacts.title')}
            aria-label={t('contacts.title')}
            aria-pressed={contactsOpen}
            className={utilityButton(contactsOpen)}
          >
            <Users size={17} />
          </button>
          {calendarEnabled && (
            <button
              type="button"
              className={utilityButton(calendarButtonPressed)}
              onClick={toggleCalendarPanel}
              title={t('calendar.panel')}
              aria-label={t('calendar.panel')}
              aria-pressed={calendarButtonPressed}
            >
              <CalendarClock size={17} />
            </button>
          )}
          {tasksEnabled && (
            <button
              type="button"
              className={`${utilityButton(tasksPanelOpen)} max-[900px]:hidden`}
              onClick={toggleTasksPanel}
              title={t('tasks.title')}
              aria-label={t('tasks.title')}
              aria-pressed={tasksPanelOpen}
            >
              <SquareCheckBig size={17} />
            </button>
          )}
          <button
            type="button"
            className={`${utilityButton(!!moreMenu)} ml-auto`}
            onClick={(e) => {
              const rect = e.currentTarget.getBoundingClientRect()
              setMoreMenu({ x: rect.right + 8, y: rect.top })
            }}
            title={t('common.more')}
            aria-label={t('common.more')}
          >
            <MoreHorizontal size={17} />
          </button>
        </div>
      )}

      {/* Bottom "more" menu: view switcher + theme settings */}
      {moreMenu && (
        <QuickSettingsMenu
          anchor={{ x: moreMenu.x, y: moreMenu.y, placement: 'up' }}
          onAddKanbanBoard={() => setBoardDialog({ mode: 'create', name: t('kanban.board.defaultName') })}
          onClose={() => setMoreMenu(null)}
        />
      )}

      {unifiedMenu && (
        <RailContextMenu x={unifiedMenu.x} y={unifiedMenu.y} onClose={() => setUnifiedMenu(null)}>
          <RailMenuItem
            icon={<EyeOff size={13} className="text-secondary" />}
            label={t('sidenav.actions.hideFromSideNav')}
            onClick={() => {
              setUnifiedInboxSideNavVisible(false)
              setUnifiedMenu(null)
            }}
          />
        </RailContextMenu>
      )}

      {calendarMenu && (
        <RailContextMenu x={calendarMenu.x} y={calendarMenu.y} onClose={() => setCalendarMenu(null)}>
          <RailMenuItem
            icon={<EyeOff size={13} className="text-secondary" />}
            label={t('sidenav.actions.hideFromSideNav')}
            onClick={() => {
              setCalendarSideNavVisible(false)
              setCalendarMenu(null)
            }}
          />
        </RailContextMenu>
      )}

      {meronMenu && (
        <RailContextMenu x={meronMenu.x} y={meronMenu.y} onClose={() => closeMeronMenu()}>
          {/* Plain buttons (the shared MenuItem), so this is a group of them
            rather than an ARIA menu: Tab moves between them, Escape closes. */}
          <div data-meron-folder-menu role="group" aria-label={meronMenu.folder.name}>
            <RailMenuItem
              className="disabled:cursor-not-allowed disabled:opacity-50"
              disabled={!meronMenu.folder.unread}
              icon={<CheckCheck size={13} className="text-secondary" />}
              label={t('meronFolders.markAllRead')}
              onClick={() => {
                const folder = meronMenu.folder
                closeMeronMenu()
                void markMeronFolderRead(folder.id)
              }}
            />
            <RailMenuItem
              icon={<Pencil size={13} className="text-secondary" />}
              label={t('meronFolders.rename')}
              onClick={() => {
                const folder = meronMenu.folder
                // The dialog takes focus; it hands it back when it closes.
                closeMeronMenu(false)
                void promptRenameMeronFolder(folder)
              }}
            />
            <RailMenuItem
              icon={
                meronMenu.folder.notify ? (
                  <BellOff size={13} className="text-secondary" />
                ) : (
                  <Bell size={13} className="text-secondary" />
                )
              }
              label={meronMenu.folder.notify ? t('folders.notify.disable') : t('folders.notify.enable')}
              onClick={() => {
                const folder = meronMenu.folder
                closeMeronMenu()
                void setMeronFolderNotify(folder, !folder.notify)
              }}
            />
            <RailMenuItem
              icon={<Trash2 size={13} />}
              label={t('meronFolders.delete')}
              danger
              onClick={() => {
                const folder = meronMenu.folder
                closeMeronMenu(false)
                void confirmDeleteMeronFolder(folder)
              }}
            />
          </div>
        </RailContextMenu>
      )}

      {menu && menuAccount && (
        <AccountContextMenu account={menuAccount} x={menu.x} y={menu.y} onClose={() => setMenu(null)} />
      )}

      {boardMenu && menuBoard && (
        <BoardContextMenu board={menuBoard} x={boardMenu.x} y={boardMenu.y} onClose={() => setBoardMenu(null)} />
      )}

      {boardDialog && (
        <BoardDialog state={boardDialog} onChange={setBoardDialog} onClose={() => setBoardDialog(null)} />
      )}
    </aside>
  )
}
