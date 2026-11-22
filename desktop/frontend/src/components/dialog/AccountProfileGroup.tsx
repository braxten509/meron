// Meron-only display name and outgoing sender name. The avatar itself
// is edited by clicking the header avatar above this group. Owns its
// own form state, seeded from the account and persisted on change.

import { useEffect, useState } from 'react'
import { useTranslation } from '../../lib/i18n'
import { setAccountName, setAccountSenderName } from '../../states/accounts'
import type { Account } from '../../types'
import { SettingsGroup, TextRow } from './AccountSettingsRows'

export function AccountProfileGroup({ account, isRSS }: { account: Account; isRSS: boolean }) {
  const { t } = useTranslation()
  const [displayNameValue, setDisplayNameValue] = useState('')
  const [senderNameValue, setSenderNameValue] = useState('')

  useEffect(() => {
    setDisplayNameValue(account.display_name || '')
    setSenderNameValue(account.sender_name || '')
  }, [account.id, account.display_name, account.sender_name])

  return (
    <SettingsGroup title={t('settings.account.profile')}>
      <TextRow
        title={t('settings.account.displayName')}
        hint={t('settings.account.displayNameHint')}
        value={displayNameValue}
        placeholder={t('settings.account.displayNamePlaceholder')}
        onChange={(value) => {
          setDisplayNameValue(value)
          void setAccountName(account.id, value)
        }}
      />
      {!isRSS && (
        <TextRow
          title={t('settings.account.senderName')}
          hint={t('settings.account.senderNameHint')}
          value={senderNameValue}
          placeholder={t('settings.account.senderNamePlaceholder')}
          onChange={(value) => {
            setSenderNameValue(value)
            void setAccountSenderName(account.id, value)
          }}
        />
      )}
    </SettingsGroup>
  )
}
