// Wails can reject backend calls with a plain string, not an Error, so callers
// should normalize unknown failures before showing them in the UI.
export function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) return error.message
  if (typeof error === 'string' && error.trim()) return error
  if (
    error &&
    typeof error === 'object' &&
    'message' in error &&
    typeof (error as { message?: unknown }).message === 'string'
  ) {
    return (error as { message: string }).message
  }
  return fallback
}

export function contextualErrorMessage(error: unknown, fallback: string): string {
  const detail = errorMessage(error, fallback)
  return detail === fallback ? fallback : `${fallback}: ${detail}`
}
