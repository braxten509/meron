// Defines the observable mailbox data shared by the desktop list and conversation
// views. Loading and mutation modules update this common state.
// Callers include calendar.ts and compose.ts.

import { observable } from '@legendapp/state'
import type { Folder, Message } from '../types'

// Mail data cache — the frontend view of the sidecar's `folders` and `messages`
// tables (threads are messages grouped by the sidecar). Ephemeral: repopulated
// from the sidecar on demand, never persisted on this side.
export const mail$ = observable({
  folders: [] as Folder[],
  // Real per-account folder lists, keyed by account id. `folders` above is the
  // view for the *selected* account (and is just a synthetic single inbox in the
  // unified view), so anything that needs a specific account's real folders —
  // e.g. the thread context menu's "Move to" in the unified inbox — reads here.
  foldersByAccount: {} as Record<string, Folder[]>,
  threads: [] as Message[],
  threadsCursor: '',
  threadAccountCursors: {} as Record<string, string>,
  threadsLoadingMore: false,
  // The view (`threadListViewKey`) whose threads are the ones in `threads`. An
  // empty list means "nothing here" only once this matches the view on screen:
  // before that the rows simply have not arrived, and saying the folder is empty
  // — at startup, or for the second or two a folder load takes — is wrong, then
  // wrong again when they land. Deliberately not a boolean set by `loadThreads`:
  // a selection or filter change repaints the list *before* the effect that
  // starts the load runs, and the client-side filter empties it on that very
  // render, so a flag set inside the load turns on a frame too late.
  threadsLoadedKey: '',
  messages: [] as Message[],
  // Opaque pagination cursor for older messages in the current thread; "" = no more.
  messagesCursor: '',
  // The thread messagesCursor pages. Right after a switch the cursor is still the
  // previous thread's, until the new thread's first page lands.
  messagesCursorThread: '',
  // Loading flag for "Load earlier messages".
  messagesLoadingMore: false,
  // True while threadRead is in flight for a newly-selected thread. The reader
  // shows a spinner (instead of the previous thread's stale messages) when this
  // is set and the loaded messages don't yet belong to the active thread —
  // notably during the on-demand ancestor fetch, which adds a network round-trip.
  threadLoading: false,
  // Thread id whose last threadRead failed (backend timeout, network down).
  // The reader shows an error + retry instead of a silent blank pane; cleared
  // on the next load attempt for that thread.
  threadErrorId: '',
  readThreads: {} as Record<string, boolean>,
})
