// Keeps the native unread indicator synchronized with frontend mailbox state.
// Updates received during an in-flight call can be coalesced.
// Imported by useAppEffects.ts.

import { invoke } from './bridge'
import { createLatestBooleanUpdater } from './latestBooleanUpdater'

type SendTrayUnread = (unread: boolean) => Promise<unknown>

export function createTrayUnreadUpdater(send: SendTrayUnread = (unread) => invoke('tray.setUnread', { unread })) {
  // Later folder refreshes still reach Go so it can repair a cleared cache.
  // Go deduplicates the native write; in-flight refreshes are coalesced here.
  return createLatestBooleanUpdater(send, { deduplicate: false })
}

export const setTrayUnread = createTrayUnreadUpdater()
