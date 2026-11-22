// Renders an account's configured picture or its fallback identity. The account
// image is distinct from the sender avatars resolved for individual messages.
// Callers include AccountSection.tsx and ColumnLabel.tsx.

import type { ComponentProps } from 'react'
import { HardDrive } from 'lucide-react'
import { isLocalAccountId } from '../../states/accounts'
import { Avatar } from './Avatar'

// Keep account-specific fallbacks consistent wherever an account is shown.
export function AccountAvatar({
  accountId,
  size = 40,
  fallback,
  ...props
}: ComponentProps<typeof Avatar> & { accountId: string }) {
  return (
    <Avatar
      {...props}
      size={size}
      fallback={isLocalAccountId(accountId) ? <HardDrive size={Math.round(size * 0.5)} /> : fallback}
    />
  )
}
