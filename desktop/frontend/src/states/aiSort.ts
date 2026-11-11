import { invoke } from '../lib/bridge'

export type AiProvider = 'openrouter' | 'tinfoil'

/** What the AI gets to read of each email. */
export type AiReads = 'headers' | 'start'

export type AiSortFolder = {
  account: string
  /** The server folder id. */
  folder: string
  /** What belongs there, in the owner's words; part of the prompt. */
  note: string
  /** Set on a Meron folder entry (this device only, every account's Inbox);
   * then `account` and `folder` are empty. */
  meron_folder_id?: string
}

export type AiSortConfig = {
  enabled: boolean
  provider: AiProvider
  /** Empty until the owner picks one: there is no default model. */
  model: string
  reads: AiReads
  folders: AiSortFolder[]
  /** The core's last few models per service, newest first. Read only: the core keeps it. */
  recent_models?: Partial<Record<AiProvider, string[]>>
}

export type AiKeyState = { has_key: boolean; key_hint: string }

export type AiErrorKind = 'key' | 'credit' | 'verify' | 'other'

export type AiSortError = {
  /** Unix seconds. */
  at: number
  message: string
  kind: AiErrorKind
}

export type AiSortSettings = {
  config: AiSortConfig
  keys: Record<AiProvider, AiKeyState>
  last_error: AiSortError | null
}

export type AiModel = {
  id: string
  name: string
  /** US dollars per million tokens; null when the service doesn't say. */
  price_in_per_m: number | null
  price_out_per_m: number | null
  structured: boolean
  /** A thinking model: slower, and its thinking counts against the answer. */
  reasoning: boolean
  /** An OpenRouter decision model: one email per request, input billed only. */
  decision?: boolean
}

export type AiSortLogState = 'moving' | 'moved' | 'left' | 'slow' | 'failed' | 'moved_back'

export type AiSortLogEntry = {
  id: number
  /** Unix seconds. */
  at: number
  account: string
  subject: string
  sender: string
  to_folder: string
  /** Set when the AI filed the mail into a Meron folder; `to_folder` then
   * holds that folder's current name. */
  meron_folder_id?: string
  /** The AI's few-word reason. */
  reason: string
  state: AiSortLogState
  detail: string
}

export type AiSortStatus = {
  log: AiSortLogEntry[]
  running: { total: number; done: number } | null
  last_error: AiSortError | null
}

export type AiSortEstimate = {
  total: number
  requests: number
  reads: AiReads
  provider: AiProvider
  model: string
  cost_usd: number | null
  accounts: { account: string; count: number }[]
}

export type AiCheckResult = { ok: boolean; kind?: AiErrorKind; message?: string }

const EMPTY_KEY: AiKeyState = { has_key: false, key_hint: '' }

export const EMPTY_AI_SORT_CONFIG: AiSortConfig = {
  enabled: false,
  provider: 'tinfoil',
  model: '',
  reads: 'headers',
  folders: [],
}

export async function loadAiSort(): Promise<AiSortSettings> {
  const res = await invoke<Partial<AiSortSettings>>('mail.aiSortConfig', {})
  return {
    config: { ...EMPTY_AI_SORT_CONFIG, ...res.config, folders: res.config?.folders ?? [] },
    keys: { openrouter: res.keys?.openrouter ?? EMPTY_KEY, tinfoil: res.keys?.tinfoil ?? EMPTY_KEY },
    last_error: res.last_error ?? null,
  }
}

// The core cleans what it keeps (notes trimmed, bad folders dropped, at most 25).
export async function saveAiSort(config: AiSortConfig): Promise<AiSortConfig> {
  const res = await invoke<{ config: AiSortConfig }>('mail.setAiSortConfig', { config })
  return { ...EMPTY_AI_SORT_CONFIG, ...res.config, folders: res.config?.folders ?? [] }
}

/** Saves a provider's key in the keychain; an empty key removes it. The key never comes back. */
export async function setAiSortKey(provider: AiProvider, key: string): Promise<AiKeyState> {
  const res = await invoke<Partial<AiKeyState>>('mail.setAiSortKey', { provider, key })
  return { has_key: res.has_key ?? false, key_hint: res.key_hint ?? '' }
}

export async function checkAiSortKey(provider: AiProvider): Promise<AiCheckResult> {
  return invoke<AiCheckResult>('mail.checkAiSortKey', { provider })
}

export async function loadAiModels(provider: AiProvider): Promise<AiModel[]> {
  const res = await invoke<{ models?: AiModel[] }>('mail.aiSortModels', { provider })
  return res.models ?? []
}

export async function estimateAiSort(): Promise<AiSortEstimate> {
  return invoke<AiSortEstimate>('mail.aiSortEstimate', {})
}

/** Starts sorting the mail already in the Inbox; the work goes on in the background. */
export async function runAiSort(): Promise<number> {
  const { count } = await invoke<{ count: number }>('mail.runAiSort', {})
  return count
}

/** Drops what is left of a "sort Inbox" run; what is done stays done. */
export async function stopAiSort(): Promise<void> {
  await invoke('mail.stopAiSort', {})
}

export async function loadAiSortStatus(): Promise<AiSortStatus> {
  const res = await invoke<Partial<AiSortStatus>>('mail.aiSortStatus', {})
  return { log: res.log ?? [], running: res.running ?? null, last_error: res.last_error ?? null }
}

export async function aiSortMoveBack(id: number): Promise<void> {
  const { ok } = await invoke<{ ok: boolean }>('mail.aiSortMoveBack', { id })
  if (!ok) throw new Error('move back failed')
}

/** An email the reader moved by hand, which AI sorting follows for mail like it. */
export type AiSortLesson = {
  id: number
  /** Unix seconds. */
  at: number
  account: string
  sender: string
  subject: string
  place: 'spam' | 'inbox' | 'meron'
  /** The Meron folder's current name, for `place: 'meron'`. */
  place_name: string
  /** False while the AI can't follow it: its Meron folder isn't on the AI's
   * list, or the account has no Spam folder. */
  used: boolean
}

export async function loadAiSortLessons(): Promise<AiSortLesson[]> {
  const res = await invoke<{ lessons?: AiSortLesson[] }>('mail.aiSortLessons', {})
  return res.lessons ?? []
}

/** Removes one lesson, here and (through its mark on the email) on the
 * reader's other devices. Answers with the list as it is now. */
export async function forgetAiSortLesson(id: number): Promise<AiSortLesson[]> {
  const res = await invoke<{ lessons?: AiSortLesson[] }>('mail.aiSortForgetLesson', { id })
  return res.lessons ?? []
}

/**
 * About what 1,000 new emails cost, one email per request (~300 tokens in, 30
 * out). A decision model reads the email and folder list (~150 tokens) and
 * bills no output.
 */
export function costPer1000(model: AiModel): number | null {
  if (model.price_in_per_m == null) return null
  if (model.decision) return (1000 * 150 * model.price_in_per_m) / 1_000_000
  if (model.price_out_per_m == null) return null
  return (1000 * (300 * model.price_in_per_m + 30 * model.price_out_per_m)) / 1_000_000
}

export function formatUsd(value: number): string {
  if (value === 0) return '$0'
  if (value < 0.01) return `$${value.toPrecision(1)}`
  return `$${value.toFixed(2)}`
}
