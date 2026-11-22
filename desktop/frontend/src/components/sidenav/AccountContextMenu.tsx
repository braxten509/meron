// Provides actions for the account selected in the navigation rail. Mailbox and
// settings operations target that account's identity.
// Imported by SideNav.tsx.

import { useValue } from '@legendapp/state/react'
import { Bell, BellOff, CheckCheck, EyeOff, Pause, Play, SlidersHorizontal } from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import { isLocalAccountId, setAccountMuted, setAccountPaused } from '../../states/accounts'
import { mail$ } from '../../states/mail'
import { markAccountInboxRead } from '../../states/mailFlags'
import { inboxUnread } from '../../states/mailFolders'
import { setAccountSideNavHidden } from '../../states/settings'
import { ui$ } from '../../states/ui'
import type { Account } from '../../types'
import { RailContextMenu, RailMenuItem } from './RailContextMenu'

const secondary = 'text-secondary'

// Right-click menu for an account avatar: settings, notifications, checking, and visibility.
export function AccountContextMenu({
  account,
  x,
  y,
  onClose,
}: {
  account: Account
  x: number
  y: number
  onClose: () => void
}) {
  const { t } = useTranslation()
  const folders = useValue(mail$.foldersByAccount[account.id])
  const hasUnread = inboxUnread(folders) > 0
  // Local folders have no inbox, nothing to notify about and nothing to check.
  const local = isLocalAccountId(account.id)
  return (
    <RailContextMenu x={x} y={y} onClose={onClose}>
      <RailMenuItem
        icon={<SlidersHorizontal size={13} className={secondary} />}
        label={t('settings.account.accountSettings')}
        onClick={() => {
          ui$.accountSettingsId.set(account.id)
          ui$.settingsOpen.set(true)
          onClose()
        }}
      />
      {!local && (
        <>
          <RailMenuItem
            className="disabled:cursor-not-allowed disabled:opacity-50"
            disabled={!hasUnread}
            icon={<CheckCheck size={13} className={secondary} />}
            label={t('threads.actions.markAllAsRead')}
            onClick={() => {
              void markAccountInboxRead(account.id)
              onClose()
            }}
          />
          <RailMenuItem
            icon={
              account.muted ? <Bell size={13} className={secondary} /> : <BellOff size={13} className={secondary} />
            }
            label={account.muted ? t('settings.account.unmuteNotifications') : t('settings.account.muteNotifications')}
            onClick={() => {
              void setAccountMuted(account.id, !(account.muted ?? false))
              onClose()
            }}
          />
          <RailMenuItem
            icon={account.paused ? <Play size={13} className={secondary} /> : <Pause size={13} className={secondary} />}
            label={account.paused ? t('settings.account.resumeChecking') : t('settings.account.pauseAccount')}
            onClick={() => {
              void setAccountPaused(account.id, !(account.paused ?? false))
              onClose()
            }}
          />
        </>
      )}
      <RailMenuItem
        icon={<EyeOff size={13} className={secondary} />}
        label={t('sidenav.actions.hideFromSideNav')}
        onClick={() => {
          setAccountSideNavHidden(account.id, true)
          onClose()
        }}
      />
    </RailContextMenu>
  )
}
