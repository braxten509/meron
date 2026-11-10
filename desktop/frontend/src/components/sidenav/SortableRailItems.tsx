import type { ReactNode } from 'react'
import { Columns3, Pause, BellOff, KeyRound, Rss } from 'lucide-react'
import { useSortable } from '@dnd-kit/sortable'
import { CSS } from '@dnd-kit/utilities'
import { useValue } from '@legendapp/state/react'
import type { DragEvent } from 'react'
import type { Account } from '../../types'
import { useTranslation } from '../../lib/i18n'
import { isLocalAccountId, unifiedAccounts } from '../../states/accounts'
import { settings$, type KanbanBoard } from '../../states/settings'
import { mail$ } from '../../states/mail'
import { inboxUnread } from '../../states/mailFolders'
import { boardUnreadCount } from '../../lib/boardUnread'
import { Avatar } from '../avatar/Avatar'
import { AccountAvatar } from '../avatar/AccountAvatar'

function accountLabel(account: { display_name: string; email: string }) {
  return account.display_name || account.email
}

function sortableStyle(
  transform: ReturnType<typeof useSortable>['transform'],
  transition: string | undefined,
  isDragging: boolean,
) {
  return {
    transform: CSS.Translate.toString(transform),
    transition: isDragging ? 'none' : transition,
    zIndex: isDragging ? 10 : undefined,
    opacity: isDragging ? 0.5 : undefined,
    pointerEvents: isDragging ? ('none' as const) : undefined,
  }
}

/** The pill at the side navigation's edge: a stub on hover, full height when
 *  active. Its wrapper must be the full-width, `relative group` row holder. */
export const railIndicator = (active: boolean) => (
  <div
    className={`absolute left-0 top-1/2 -translate-y-1/2 w-1 rounded-r bg-accent transition-[height] duration-200 ease-out ${
      active ? 'h-6' : 'h-0 group-hover:h-4'
    }`}
  />
)

/**
 * The geometry every side navigation row shares: one fixed height, a 20px
 * icon slot and a 20px text line, so icons and names sit on the same centre
 * line and no row is taller than another. The text never changes weight with
 * state, so selecting a row moves nothing; only colours change.
 */
export const NAV_ROW_HEIGHT = 'h-8'

/** A count at a row's end, in the badge colours the rail always used. */
export function NavCount({ count }: { count: number }) {
  if (count <= 0) return null
  return (
    <span className="flex h-[1.125rem] min-w-[1.125rem] shrink-0 items-center justify-center rounded-full bg-accent-badge px-1.5 text-[0.625rem] font-semibold leading-none tabular-nums text-white">
      {count > 9999 ? '9999+' : count}
    </span>
  )
}

/** One row's look: icon, name, optional trailing marks, count. */
export function NavRow({
  active,
  icon,
  label,
  count = 0,
  trailing,
  quiet = false,
}: {
  active: boolean
  icon: ReactNode
  label: ReactNode
  count?: number
  trailing?: ReactNode
  /** A secondary action row ("+ New …"), drawn in the muted ink. */
  quiet?: boolean
}) {
  return (
    <div
      className={`flex ${NAV_ROW_HEIGHT} w-full items-center gap-2.5 rounded-lg px-2 text-[0.8125rem] leading-5 transition-colors duration-150 ${
        active
          ? 'bg-accent/15 text-sidenav-ink'
          : quiet
            ? 'text-sidenav-ink/55 group-hover:bg-sidenav-ink/[0.07] group-hover:text-sidenav-ink'
            : 'text-sidenav-ink/80 group-hover:bg-sidenav-ink/[0.07] group-hover:text-sidenav-ink'
      }`}
    >
      <span className="flex h-5 w-5 shrink-0 items-center justify-center">{icon}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {trailing}
      <NavCount count={count} />
    </div>
  )
}

/** A section heading ("MERON FOLDERS"), its text on the rows' icon edge. */
export function NavSectionHeading({ children, first = false }: { children: ReactNode; first?: boolean }) {
  return (
    <div
      className={`flex h-6 shrink-0 items-center px-4 text-[0.6875rem] font-semibold uppercase leading-4 tracking-[0.06em] text-sidenav-ink/50 ${
        first ? '' : 'mt-3'
      }`}
    >
      <span className="truncate">{children}</span>
    </div>
  )
}

interface SortableBoardProps {
  shortcut?: string
  board: Pick<KanbanBoard, 'id' | 'name' | 'avatarUrl' | 'columns' | 'showUnreadBadge'>
  active: boolean
  onSelect: () => void
  onContextMenu: (e: React.MouseEvent) => void
}

// The board's unread count when its badge is switched on, else 0. Counts come
// from the folder caches, which useAppEffects keeps filled and synced while the
// badge is on, so a board that isn't open has a current number too. A row with
// its badge off subscribes to nothing, and one with it on re-renders only when
// the number changes.
function useBoardUnread(board: SortableBoardProps['board']): number {
  return useValue(() =>
    board.showUnreadBadge === true
      ? boardUnreadCount(
          board.columns,
          mail$.foldersByAccount.get(),
          unifiedAccounts().map((account) => account.id),
        )
      : 0,
  )
}

export function SortableBoard({ board, active, onSelect, onContextMenu, shortcut }: SortableBoardProps) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: board.id })
  const unreadCount = useBoardUnread(board)

  return (
    <div
      ref={setNodeRef}
      style={sortableStyle(transform, transition, isDragging) as any}
      {...attributes}
      {...listeners}
      onClick={onSelect}
      onContextMenu={onContextMenu}
      className="relative w-full px-2 group cursor-pointer"
      title={board.name + (shortcut ? ` (${shortcut})` : '')}
    >
      {railIndicator(active)}
      <NavRow
        active={active}
        icon={
          board.avatarUrl ? (
            <Avatar name={board.name} src={board.avatarUrl} size={18} className="pointer-events-none" />
          ) : (
            <Columns3 size={16} />
          )
        }
        label={board.name}
        count={unreadCount}
      />
    </div>
  )
}

interface SortableAccountProps {
  shortcut?: string
  account: Account
  active: boolean
  onSelect: () => void
  onContextMenu: (e: React.MouseEvent) => void
  onFeedDragOver?: (e: DragEvent<HTMLDivElement>, account: Account) => void
  onFeedDrop?: (e: DragEvent<HTMLDivElement>, account: Account) => void
}

export function SortableAccount({
  account,
  shortcut,
  active,
  onSelect,
  onContextMenu,
  onFeedDragOver,
  onFeedDrop,
}: SortableAccountProps) {
  const { t } = useTranslation()
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: account.id })

  const isRSS = account.provider === 'rss' || account.auth_type === 'rss'
  const isLocal = isLocalAccountId(account.id)
  const needsReconnect = account.needs_reconnect === true
  const isPaused = account.paused ?? false
  const isMuted = account.muted ?? false
  const showUnreadBadge = useValue(settings$.showUnreadAccountBadge)
  const accountFolders = useValue(mail$.foldersByAccount[account.id])
  const unreadCount = showUnreadBadge ? inboxUnread(accountFolders) : 0
  const name = isRSS
    ? account.display_name || 'RSS Feeds'
    : isLocal
      ? account.display_name || t('accounts.local.name')
      : accountLabel(account)
  const baseTooltip = isRSS
    ? name
    : isLocal
      ? `${name} (${t('accounts.local.subtitle')})`
      : account.display_name
        ? `${account.display_name} (${account.email})`
        : account.email
  const stateSuffix = needsReconnect ? ' — Needs reconnect' : isPaused ? ' — Paused' : isMuted ? ' — Muted' : ''

  return (
    <div
      ref={setNodeRef}
      style={sortableStyle(transform, transition, isDragging) as any}
      {...attributes}
      {...listeners}
      onClick={onSelect}
      onContextMenu={onContextMenu}
      onDragOver={(event) => onFeedDragOver?.(event, account)}
      onDrop={(event) => onFeedDrop?.(event, account)}
      className="relative w-full px-2 group cursor-pointer"
      title={baseTooltip + stateSuffix + (shortcut ? ` (${shortcut})` : '')}
    >
      {railIndicator(active)}
      <NavRow
        active={active}
        icon={
          <AccountAvatar
            accountId={account.id}
            name={accountLabel(account)}
            src={account.avatar_url}
            size={18}
            fallback={isRSS ? <Rss size={11} /> : undefined}
            className={`pointer-events-none transition-all ${isPaused || needsReconnect ? 'grayscale opacity-40' : ''}`}
          />
        }
        label={<span className={isPaused || needsReconnect ? 'opacity-60' : undefined}>{name}</span>}
        count={unreadCount}
        trailing={
          (needsReconnect || isPaused || isMuted) && (
            <span
              className={`flex h-4 w-4 shrink-0 items-center justify-center ${
                needsReconnect ? 'text-amber-600' : 'text-sidenav-ink/50'
              }`}
              title={needsReconnect ? 'Needs reconnect' : isPaused ? 'Paused' : 'Muted'}
            >
              {needsReconnect ? (
                <KeyRound size={12} />
              ) : isPaused ? (
                <Pause size={11} className="fill-current" />
              ) : (
                <BellOff size={11} />
              )}
            </span>
          )
        }
      />
    </div>
  )
}
