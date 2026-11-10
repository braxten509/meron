import { invoke } from '../lib/bridge'
import { boot } from '../boot'
import { ui$ } from './ui'
import { loadThreads } from './mail'
import { loadFolders } from './mailFolders'

/** One side's counts, as the core reports them. */
export type SetupCounts = { accounts: number; folders: number; keys: number }

export type SetupDevice = 'phone' | 'computer'

/** `setupCopy.status`: see meron-core's `setup_copy` module. */
export type SetupCopyStatus = {
  active: boolean
  stage: 'idle' | 'waiting' | 'connecting' | 'connected' | 'copying' | 'applying' | 'done' | 'failed'
  role: '' | 'host' | 'guest'
  me: SetupDevice | ''
  peer_device: SetupDevice | ''
  qr: string
  qr_svg: string
  address: string
  code: string
  expires_at: number
  now: number
  keep: SetupDevice | ''
  mine: SetupCounts
  peer: SetupCounts | null
  starting: boolean
  error: string
  attempts_left: number
  replaced: boolean
  result: { accounts?: number; folders?: number; keys?: number; secret_failures?: number } | null
}

export type SafetyInfo = { exists: boolean; created_at?: number; counts?: SetupCounts }

export const setupCopyStatus = () => invoke<SetupCopyStatus>('setupCopy.status')
export const startSetupCopy = () => invoke<SetupCopyStatus>('setupCopy.host')
export const setSetupKeep = (keep: SetupDevice) => invoke<SetupCopyStatus>('setupCopy.setKeep', { keep })
export const runSetupCopy = () => invoke<SetupCopyStatus>('setupCopy.copy')
export const cancelSetupCopy = () => invoke<SetupCopyStatus>('setupCopy.cancel')
export const safetyCopyInfo = () => invoke<SafetyInfo>('setupCopy.safetyInfo')

/**
 * Put back the setup the last copy replaced. What is here now becomes the
 * safety copy. Re-boots, as everything loaded is stale.
 */
export async function restoreSafetyCopy(): Promise<void> {
  await invoke('setupCopy.restoreSafety')
  await reloadAfterReplace()
}

/** This device's setup was replaced: every loaded view is stale. */
export async function reloadAfterReplace(): Promise<void> {
  await boot()
  await Promise.all([loadThreads(false), loadFolders(ui$.selectedAccount.get(), false)]).catch(() => {})
}
