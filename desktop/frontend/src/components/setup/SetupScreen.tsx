// Presents the desktop onboarding screen before the mailbox can be used. The
// available actions lead into the account or configuration setup flow.
// Imported by App.tsx.

import { AccountDialog } from '../dialog/AccountDialog'

export function SetupScreen() {
  return (
    <main className="flex h-full w-full justify-center overflow-y-auto bg-app px-6 py-10 text-primary">
      <AccountDialog variant="setup" />
    </main>
  )
}
