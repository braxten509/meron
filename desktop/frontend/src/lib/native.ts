// Routes external links and clipboard writes to the Wails
// runtime. Browser fallbacks support web contexts, and the native
// clipboard path also works when a message iframe has focus.

// Native shell helpers that route through the Wails runtime.

// Open a URL in the user's default browser. A bare `<a target="_blank">` does
// nothing inside the Wails webview (it tries to navigate the app frame), so we
// route through the runtime's BrowserOpenURL.
export function openExternal(url: string) {
  const open = (window as any).runtime?.BrowserOpenURL
  if (open) {
    open(url)
  } else {
    window.open(url, '_blank', 'noreferrer')
  }
}

// Put text on the system clipboard. navigator.clipboard rejects when the top
// document isn't focused — e.g. a link right-clicked inside a message iframe
// keeps focus in that frame — so prefer the runtime's native clipboard.
export async function copyText(text: string) {
  const set = (window as any).runtime?.ClipboardSetText
  if (set && (await set(text).catch(() => false))) return
  await navigator.clipboard?.writeText(text)
}
