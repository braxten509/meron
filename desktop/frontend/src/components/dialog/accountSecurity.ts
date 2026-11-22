// Resolves transport-security choices in the account editor after a port change.
// Explicit user selections must survive changes to inferred defaults.
// Imported by AccountDialogCustom.tsx and useAccountDialog.ts.

export type MailSecurity = 'tls' | 'starttls' | 'none'

export function securityForPort(port: number): MailSecurity {
  if (port === 25 || port === 143 || port === 587) return 'starttls'
  if (port === 3143 || port === 3587) return 'none'
  return 'tls'
}

export function securityAfterPortEdit(current: MailSecurity, touched: boolean, portText: string): MailSecurity {
  if (touched) return current
  const port = Number(portText)
  return port > 0 ? securityForPort(port) : current
}

export function serverSelectionAfterDiscovery(
  currentPort: string,
  currentSecurity: MailSecurity,
  securityTouched: boolean,
  hostTouched: boolean,
  portTouched: boolean,
  discoveredPort: number,
): { port: string; security: MailSecurity } {
  if (hostTouched || portTouched || discoveredPort <= 0) {
    return { port: currentPort, security: currentSecurity }
  }
  return {
    port: String(discoveredPort),
    security: securityTouched ? currentSecurity : securityForPort(discoveredPort),
  }
}
