// Identifies the desktop platform for browser-side behavior.
// Shared checks for macOS and Linux keep host-specific
// controls and shortcuts consistent across components.

// Keep platform detection shared by shortcuts and native appearance choices.
export function isMacPlatform(userAgent: string, platform = ''): boolean {
  return /mac|iphone|ipad|ipod/i.test(userAgent + ' ' + platform)
}

export function isLinuxPlatform(userAgent: string, platform = ''): boolean {
  return /linux/i.test(userAgent + ' ' + platform)
}
