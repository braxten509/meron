// The app-level certificate prompt: raised when an action on an existing account hits
// a server certificate we cannot validate — today, a send whose submission server was
// refused. Setup-time failures are handled inline by the account dialog instead, where
// the servers are already on screen.

import { useValue } from '@legendapp/state/react'
import { Dialog } from './Dialog'
import { useTranslation } from '../../lib/i18n'
import { certTrust$, dismissCertificatePrompt, trustPromptedCertificate } from '../../states/certificateTrust'
import { CertificateTrustPanel } from './CertificateTrustPanel'

export function CertificateTrustDialog() {
  const { t } = useTranslation()
  const prompt = useValue(certTrust$.prompt)
  const busy = useValue(certTrust$.busy)

  if (!prompt) return null

  return (
    <Dialog
      onClose={dismissCertificatePrompt}
      dismissible={!busy}
      role="alertdialog"
      aria-label={t('accounts.certificate.title')}
      backdropClassName="z-[120] bg-black/45 dark:bg-black/45 backdrop-blur-none"
      className="max-w-md rounded-xl"
    >
      <CertificateTrustPanel
        prompt={prompt}
        busy={busy}
        onTrust={trustPromptedCertificate}
        onDismiss={dismissCertificatePrompt}
      />
    </Dialog>
  )
}
