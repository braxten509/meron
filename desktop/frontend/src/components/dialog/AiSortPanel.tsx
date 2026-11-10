import { useEffect, useRef, useState } from 'react'
import { ChevronDown, ChevronLeft, ChevronRight, Eye, EyeOff, Play, ShieldCheck, Square, Undo2, X } from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import {
  aiSortMoveBack,
  checkAiSortKey,
  costPer1000,
  EMPTY_AI_SORT_CONFIG,
  estimateAiSort,
  formatUsd,
  loadAiModels,
  loadAiSort,
  loadAiSortStatus,
  runAiSort,
  saveAiSort,
  setAiSortKey,
  stopAiSort,
  type AiErrorKind,
  type AiModel,
  type AiProvider,
  type AiReads,
  type AiSortConfig,
  type AiSortFolder,
  type AiSortLogEntry,
  type AiSortSettings,
  type AiSortStatus,
} from '../../states/aiSort'
import { confirmAction, showToast } from '../../states/ui'
import { useValue } from '@legendapp/state/react'
import { isMeronArchive, meronFolders$ } from '../../states/meronFolders'
import type { Folder } from '../../types'
import { Button } from '../button/Button'
import { IconButton } from '../button/IconButton'
import { TextInput } from '../field/Field'
import { FloatingContextMenu } from '../menu/FloatingContextMenu'
import { MenuItem } from '../menu/MenuItem'
import { flyoutClass } from '../threads/ThreadMoveCopyItems'
import { SegmentedRow, SettingRow, SettingsGroup, Switch } from './AccountSettingsRows'
import { RuleFolderPicker, useRuleTargetLabel } from './RuleFolderPicker'

const RUN_POLL_MS = 1500
// History rows shown at first and added by each "View more".
const LOG_PAGE = 50
const MAX_FOLDERS = 25
const MAX_NOTE = 120
const MODELS_SHOWN = 100

const PROVIDER_NAMES: Record<AiProvider, string> = { tinfoil: 'Tinfoil', openrouter: 'OpenRouter' }

const ERROR_KEYS: Record<AiErrorKind, string> = {
  key: 'aiSort.errorKey',
  credit: 'aiSort.errorCredit',
  verify: 'aiSort.errorVerify',
  other: 'aiSort.errorOther',
}

// The core refuses these as targets too; the picker greys them out.
const SYSTEM_ROLES = new Set(['inbox', 'sent', 'drafts', 'trash', 'junk'])
const isSystemFolder = (folder: Folder) => SYSTEM_ROLES.has(folder.role) || folder.id.toLowerCase() === 'inbox'

// A Meron folder entry (this device only) has no account or server folder;
// its id keys it instead.
type FolderKeyInput = Pick<AiSortFolder, 'account' | 'folder' | 'meron_folder_id'>
const folderKey = ({ account, folder, meron_folder_id }: FolderKeyInput) =>
  meron_folder_id ? `meron\u0000${meron_folder_id}` : `${account}\u0000${folder}`

// Lets the AI move new mail into the owner's own folders before it shows in the
// Inbox. The key goes to the keychain and never comes back; the rest is saved
// whole, one save at a time, like Rules.
export function AiSortPanel() {
  const { t } = useTranslation()
  const targetLabel = useRuleTargetLabel()
  const meronFolders = useValue(meronFolders$.list)
  const meronName = (id: string) => meronFolders.find((folder) => folder.id === id)?.name ?? id
  const [settings, setSettings] = useState<AiSortSettings | null>(null)
  const [config, setConfig] = useState<AiSortConfig>(EMPTY_AI_SORT_CONFIG)
  const [status, setStatus] = useState<AiSortStatus>({ log: [], running: null, last_error: null })
  // Notes being typed, kept apart until the field is left.
  const [notes, setNotes] = useState<Record<string, string>>({})
  const [keyInput, setKeyInput] = useState('')
  const [showKey, setShowKey] = useState(false)
  // The service whose key box is open.
  const [editingKeyFor, setEditingKeyFor] = useState<AiProvider | null>(null)
  const [keyBusy, setKeyBusy] = useState(false)
  const [starting, setStarting] = useState(false)
  const [movingBack, setMovingBack] = useState<number | null>(null)
  const [models, setModels] = useState<Partial<Record<AiProvider, AiModel[]>>>({})
  const [modelsFailed, setModelsFailed] = useState<Partial<Record<AiProvider, boolean>>>({})
  const [modelInput, setModelInput] = useState<string | null>(null)
  const [logOpen, setLogOpen] = useState(false)
  const [logShown, setLogShown] = useState(LOG_PAGE)
  const saving = useRef(false)
  const queued = useRef<AiSortConfig | null>(null)

  const reload = () =>
    void loadAiSort().then(
      (loaded) => {
        setSettings(loaded)
        // A save on its way answers with the newer config itself.
        if (!saving.current) setConfig(loaded.config)
      },
      () => setSettings(null),
    )

  useEffect(reload, [])

  const refreshStatus = () => void loadAiSortStatus().then(setStatus, () => {})

  useEffect(refreshStatus, [])

  const running = status.running
  useEffect(() => {
    if (!running) return
    const timer = setInterval(refreshStatus, RUN_POLL_MS)
    return () => clearInterval(timer)
  }, [running !== null])

  // Shows the change at once; a change made while a save is on its way goes
  // out after it, built on top of it.
  const persist = async (next: AiSortConfig) => {
    setConfig(next)
    if (saving.current) {
      queued.current = next
      return
    }
    saving.current = true
    try {
      let current: AiSortConfig | null = next
      while (current) {
        queued.current = null
        try {
          const saved = await saveAiSort(current)
          if (!queued.current) setConfig(saved)
        } catch {
          showToast(t('aiSort.saveFailed'), 'error')
          queued.current = null
          reload()
        }
        current = queued.current
      }
    } finally {
      saving.current = false
    }
  }

  const loadModels = (provider: AiProvider) => {
    if (models[provider]) return
    setModelsFailed((failed) => ({ ...failed, [provider]: false }))
    void loadAiModels(provider).then(
      (list) => setModels((all) => ({ ...all, [provider]: list })),
      () => setModelsFailed((failed) => ({ ...failed, [provider]: true })),
    )
  }

  if (!settings) {
    return (
      <div className="flex flex-col gap-4">
        <p className="text-xs text-secondary">{t('common.loading')}</p>
      </div>
    )
  }

  const provider = config.provider
  const keyState = settings.keys[provider]
  const missing = [
    !keyState.has_key && t('aiSort.needsKey'),
    !config.model.trim() && t('aiSort.needsModel'),
    config.folders.length === 0 && t('aiSort.needsFolder'),
  ].filter((item): item is string => Boolean(item))
  const ready = missing.length === 0
  const lastError = status.last_error ?? settings.last_error
  const chosenModel = models[provider]?.find((model) => model.id === config.model)

  const toggleEnabled = () => {
    if (config.enabled) void persist({ ...config, enabled: false })
    else if (ready) void persist({ ...config, enabled: true })
  }

  // Model ids differ between services: a switch brings back the model last
  // used on the new one, and stays on only when that service is ready too.
  const pickProvider = (next: AiProvider) => {
    if (next === provider) return
    setModelInput(null)
    const model = config.recent_models?.[next]?.[0] ?? ''
    const enabled = config.enabled && model !== '' && settings.keys[next]?.has_key === true
    void persist({ ...config, provider: next, model, enabled })
  }

  const openKeyBox = (forProvider: AiProvider | null) => {
    setEditingKeyFor(forProvider)
    setKeyInput('')
    setShowKey(false)
  }

  const saveKey = async (forProvider: AiProvider, key: string) => {
    setKeyBusy(true)
    try {
      const state = await setAiSortKey(forProvider, key)
      setSettings({ ...settings, keys: { ...settings.keys, [forProvider]: state } })
      openKeyBox(null)
      // A list fetched with no key, or with the old one, may differ.
      setModels((all) => ({ ...all, [forProvider]: undefined }))
    } catch {
      showToast(t('aiSort.saveFailed'), 'error')
    } finally {
      setKeyBusy(false)
      // The core turns sorting off when the active key goes, so take its
      // config (and key states) instead of guessing.
      reload()
    }
  }

  const checkKey = async (forProvider: AiProvider) => {
    setKeyBusy(true)
    try {
      const result = await checkAiSortKey(forProvider)
      if (result.ok) showToast(t('aiSort.keyOk'))
      else showToast(t(ERROR_KEYS[result.kind ?? 'other']), 'error')
    } catch {
      showToast(t('aiSort.errorOther'), 'error')
    } finally {
      setKeyBusy(false)
      refreshStatus()
    }
  }

  const addFolder = (account: string, folder: string) => {
    if (config.folders.length >= MAX_FOLDERS) return
    if (config.folders.some((item) => folderKey(item) === folderKey({ account, folder }))) return
    void persist({ ...config, folders: [...config.folders, { account, folder, note: '' }] })
  }

  const addMeronFolder = (id: string) => {
    if (config.folders.length >= MAX_FOLDERS) return
    if (config.folders.some((item) => item.meron_folder_id === id)) return
    void persist({
      ...config,
      folders: [...config.folders, { account: '', folder: '', meron_folder_id: id, note: '' }],
    })
  }

  const removeFolder = (target: AiSortFolder) => {
    const key = folderKey(target)
    setNotes(({ [key]: _, ...rest }) => rest)
    const folders = config.folders.filter((item) => folderKey(item) !== key)
    void persist({ ...config, folders, enabled: folders.length > 0 && config.enabled })
  }

  const saveNote = (target: AiSortFolder) => {
    const key = folderKey(target)
    const typed = notes[key]
    setNotes(({ [key]: _, ...rest }) => rest)
    if (typed === undefined || typed.trim() === target.note) return
    void persist({
      ...config,
      folders: config.folders.map((item) => (folderKey(item) === key ? { ...item, note: typed.trim() } : item)),
    })
  }

  const saveModelInput = () => {
    if (modelInput === null) return
    const model = modelInput.trim()
    setModelInput(null)
    if (model !== config.model) void persist({ ...config, model, enabled: model ? config.enabled : false })
  }

  // Always asks: it sends mail to the AI service and costs money.
  const sortInbox = async () => {
    setStarting(true)
    try {
      const estimate = await estimateAiSort()
      if (estimate.total === 0) {
        showToast(t('aiSort.sortInboxNothing'))
        return
      }
      const name = PROVIDER_NAMES[estimate.provider] ?? estimate.provider
      const confirmed = await confirmAction({
        title: t('aiSort.sortInbox'),
        message:
          estimate.cost_usd == null
            ? t('aiSort.sortInboxConfirmNoPrice', { count: estimate.total, provider: name })
            : t('aiSort.sortInboxConfirm', {
                count: estimate.total,
                provider: name,
                cost: formatUsd(estimate.cost_usd),
              }),
        confirmLabel: t('aiSort.sortInboxAction'),
        cancelLabel: t('buttons.cancel'),
      })
      if (!confirmed) return
      showToast(t('aiSort.sortInboxStarted', { count: await runAiSort() }))
    } catch {
      showToast(t('aiSort.sortInboxFailed'), 'error')
    } finally {
      setStarting(false)
      refreshStatus()
    }
  }

  const stop = async () => {
    await stopAiSort().catch(() => showToast(t('aiSort.sortInboxFailed'), 'error'))
    refreshStatus()
  }

  const moveBack = async (entry: AiSortLogEntry) => {
    setMovingBack(entry.id)
    try {
      await aiSortMoveBack(entry.id)
    } catch {
      showToast(t('aiSort.moveBackFailed'), 'error')
    } finally {
      setMovingBack(null)
      refreshStatus()
    }
  }

  const logLabel = (entry: AiSortLogEntry) => {
    const folder = targetLabel({ accountId: entry.account, folderId: entry.to_folder })
    switch (entry.state) {
      case 'moving':
      case 'moved':
        // The core already answers with the Meron folder's current name.
        return entry.meron_folder_id
          ? t('aiSort.logMovedMeron', { folder: entry.to_folder })
          : t('aiSort.logMoved', { folder })
      case 'moved_back':
        return t('aiSort.logMovedBack')
      case 'left':
        return t('aiSort.logLeft')
      case 'slow':
        return t('aiSort.logSlow')
      default:
        return t('aiSort.logFailed')
    }
  }


  const tagLabel = (entry: AiSortLogEntry) => {
    switch (entry.state) {
      case 'moving':
      case 'moved':
        return entry.meron_folder_id
          ? entry.to_folder
          : targetLabel({ accountId: entry.account, folderId: entry.to_folder })
      case 'moved_back':
        return t('aiSort.tagMovedBack')
      case 'left':
        return t('aiSort.tagLeft')
      case 'slow':
        return t('aiSort.tagSlow')
      default:
        return t('aiSort.tagFailed')
    }
  }

  if (logOpen) {
    return (
      <div className="flex flex-col gap-3">
        <div className="flex items-center gap-2">
          <IconButton icon={ChevronLeft} iconSize={14} label={t('buttons.back')} size="sm" onClick={() => setLogOpen(false)} />
          <h3 className="text-sm font-semibold text-primary">{t('aiSort.log')}</h3>
        </div>
        <div className="divide-y divide-border/40 overflow-hidden rounded-2xl border border-border/60 bg-raised/80 shadow-sm shadow-black/[0.03] dark:shadow-black/10">
          {status.log.length === 0 && <p className="px-3.5 py-2.5 text-xs text-secondary">{t('aiSort.logEmpty')}</p>}
          {status.log.slice(0, logShown).map((entry) => {
            const moved = entry.state === 'moved' || entry.state === 'moving'
            const failed = entry.state === 'failed'
            const tagClass = failed
              ? 'bg-rose-500/10 text-rose-500'
              : moved
                ? 'bg-accent/10 text-accent'
                : 'bg-secondary/10 text-secondary'
            return (
              <div
                key={entry.id}
                className="flex min-h-10 items-center gap-2 py-1 pr-1.5 pl-3.5 text-xs"
                title={[logLabel(entry), entry.sender, entry.reason, new Date(entry.at * 1000).toLocaleString()]
                  .filter(Boolean)
                  .join(' · ')}
              >
                <span className={`min-w-0 flex-1 truncate ${moved ? 'text-primary' : 'text-secondary'}`}>
                  {entry.subject || entry.sender}
                </span>
                <span className={`max-w-40 shrink-0 truncate rounded-full px-2 text-[0.625rem] font-medium leading-5 ${tagClass}`}>
                  {tagLabel(entry)}
                </span>
                {/* The same room on every line, so the tags line up. */}
                <span className="flex w-7 shrink-0 justify-center">
                  {entry.state === 'moved' && (
                    <IconButton
                      icon={Undo2}
                      iconSize={13}
                      label={t('aiSort.moveBack')}
                      size="sm"
                      disabled={movingBack !== null}
                      onClick={() => void moveBack(entry)}
                    />
                  )}
                </span>
              </div>
            )
          })}
        </div>
        {status.log.length > logShown && (
          <Button variant="ghost" size="sm" onClick={() => setLogShown(logShown + LOG_PAGE)}>
            {t('aiSort.viewMore')}
          </Button>
        )}
      </div>
    )
  }

  return (
    <div className="flex flex-col gap-4">

      {lastError && (
        <p
          className="rounded-xl border border-rose-500/25 bg-rose-500/[0.06] px-3.5 py-2.5 text-[0.75rem] font-medium text-rose-500"
          title={lastError.message || undefined}
        >
          {t(ERROR_KEYS[lastError.kind] ?? ERROR_KEYS.other)}
        </p>
      )}

      <SettingsGroup title={t('aiSort.title')}>
        <SettingRow
          title={t('aiSort.enabled')}
          control={
            <span className={!config.enabled && !ready ? 'opacity-50' : ''} aria-disabled={!config.enabled && !ready}>
              <Switch checked={config.enabled} label={t('aiSort.enabled')} onChange={toggleEnabled} />
            </span>
          }
        />
        <SegmentedRow<AiReads>
          title={t('aiSort.reads')}
          value={config.reads}
          options={[
            { value: 'headers', label: t('aiSort.readsHeaders') },
            { value: 'start', label: t('aiSort.readsStart') },
          ]}
          onChange={(reads) => void persist({ ...config, reads })}
        />
      </SettingsGroup>

      {/* One row per service with its own key, so switching is one click. */}
      <SettingsGroup title={t('aiSort.provider')}>
        {(['tinfoil', 'openrouter'] as const).map((option) => {
          const state = settings.keys[option]
          const chosen = provider === option
          return (
            <div key={option}>
              <div className="flex min-h-12 items-center gap-3 px-3.5 py-2">
                <button
                  type="button"
                  role="radio"
                  aria-checked={chosen}
                  onClick={() => pickProvider(option)}
                  className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 text-left"
                >
                  <span
                    className={`h-4 w-4 shrink-0 rounded-full border-2 ${chosen ? 'border-[5px] border-accent' : 'border-secondary/60'}`}
                  />
                  <span className="flex min-w-0 flex-col">
                    <span className="truncate text-xs font-medium text-primary">
                      {t(option === 'tinfoil' ? 'aiSort.providerTinfoil' : 'aiSort.providerOpenRouter')}
                    </span>
                    <span className="truncate text-[0.6875rem] text-secondary">
                      {state?.has_key ? t('aiSort.keySaved', { hint: state.key_hint }) : t('aiSort.noKey')}
                    </span>
                  </span>
                </button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={keyBusy}
                  onClick={() => openKeyBox(editingKeyFor === option ? null : option)}
                >
                  {t(state?.has_key ? 'aiSort.editKey' : 'aiSort.addKey')}
                </Button>
              </div>
              {editingKeyFor === option && (
                <div className="flex flex-wrap items-center gap-2 px-3.5 pb-2.5">
                  <span className="relative flex min-w-48 flex-1 items-center">
                    <TextInput
                      autoFocus
                      type={showKey ? 'text' : 'password'}
                      value={keyInput}
                      placeholder={t('aiSort.keyPlaceholder')}
                      autoComplete="off"
                      spellCheck={false}
                      aria-label={t('aiSort.key')}
                      onChange={(event) => setKeyInput(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' && keyInput.trim() && !keyBusy) {
                          event.preventDefault()
                          void saveKey(option, keyInput.trim())
                        }
                        if (event.key === 'Escape') openKeyBox(null)
                      }}
                      className="w-full pr-9 font-semibold"
                    />
                    <button
                      type="button"
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={() => setShowKey((value) => !value)}
                      className="absolute right-1.5 flex h-6 w-6 cursor-pointer items-center justify-center rounded-lg text-secondary transition-colors hover:bg-hover hover:text-primary"
                      aria-label={showKey ? t('accounts.actions.hidePassword') : t('accounts.actions.showPassword')}
                    >
                      {showKey ? <EyeOff size={13} /> : <Eye size={13} />}
                    </button>
                  </span>
                  <Button size="sm" disabled={!keyInput.trim() || keyBusy} onClick={() => void saveKey(option, keyInput.trim())}>
                    {t('buttons.save')}
                  </Button>
                  {state?.has_key && (
                    <>
                      <Button
                        variant="secondary"
                        size="sm"
                        leftIcon={ShieldCheck}
                        disabled={keyBusy}
                        onClick={() => void checkKey(option)}
                      >
                        {t('aiSort.keyCheck')}
                      </Button>
                      <Button variant="danger" size="sm" disabled={keyBusy} onClick={() => void saveKey(option, '')}>
                        {t('aiSort.keyRemove')}
                      </Button>
                    </>
                  )}
                </div>
              )}
            </div>
          )
        })}
      </SettingsGroup>

      <SettingsGroup title={t('aiSort.model')}>
        <SettingRow
          title={t('aiSort.model')}
          control={
            <ModelPicker
              value={config.model}
              recent={config.recent_models?.[provider] ?? []}
              label={chosenModel?.name || config.model || t('aiSort.modelPick')}
              models={models[provider]}
              failed={Boolean(modelsFailed[provider])}
              onOpen={() => loadModels(provider)}
              onPick={(model) => void persist({ ...config, model })}
            />
          }
        />
        {chosenModel && <ModelFacts model={chosenModel} className="px-3.5 py-2 text-[0.6875rem] text-secondary" />}
        {modelsFailed[provider] && (
          <div className="flex min-h-11 items-center justify-between gap-4 px-3.5 py-2">
            <span className="min-w-0 truncate text-[0.6875rem] font-medium text-rose-500">
              {t('aiSort.modelsFailed')}
            </span>
            <TextInput
              value={modelInput ?? config.model}
              placeholder={t('aiSort.modelId')}
              aria-label={t('aiSort.modelId')}
              spellCheck={false}
              onChange={(event) => setModelInput(event.target.value)}
              onBlur={saveModelInput}
              onKeyDown={(event) => {
                if (event.key === 'Enter') saveModelInput()
              }}
              className="w-44 font-semibold"
            />
          </div>
        )}
      </SettingsGroup>

      <SettingsGroup title={t('aiSort.folders')} overflowVisible>
        {config.folders.map((item) => {
          const key = folderKey(item)
          return (
            <div key={key} className="flex min-h-11 items-center gap-3 px-3.5 py-2">
              {item.meron_folder_id ? (
                <span className="flex w-40 shrink-0 items-center gap-1.5">
                  <span className="min-w-0 truncate text-xs font-medium text-primary">
                    {meronName(item.meron_folder_id)}
                  </span>
                  <span className="shrink-0 rounded bg-secondary/10 px-1 text-[0.625rem] font-medium leading-4 text-secondary">
                    {t('aiSort.meronFolderTag')}
                  </span>
                </span>
              ) : (
                <span className="w-40 shrink-0 truncate text-xs font-medium text-primary">
                  {targetLabel({ accountId: item.account, folderId: item.folder })}
                </span>
              )}
              <TextInput
                value={notes[key] ?? item.note}
                maxLength={MAX_NOTE}
                placeholder={t('aiSort.notePlaceholder')}
                aria-label={t('aiSort.notePlaceholder')}
                onChange={(event) => setNotes({ ...notes, [key]: event.target.value })}
                onBlur={() => saveNote(item)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') event.currentTarget.blur()
                }}
                className="flex-1"
              />
              <IconButton
                icon={X}
                iconSize={13}
                label={t('aiSort.removeFolder')}
                size="sm"
                onClick={() => removeFolder(item)}
              />
            </div>
          )
        })}
        {config.folders.length < MAX_FOLDERS && (
          <SettingRow
            title={t('aiSort.addFolder')}
            control={
              <RuleFolderPicker
                label={t('aiSort.addFolder')}
                value={t('aiSort.addFolder')}
                options={[]}
                exclude={isSystemFolder}
                onPick={({ accountId, folderId }) => addFolder(accountId, folderId)}
                meronFolders={meronFolders.filter((folder) => !isMeronArchive(folder))}
                chosenMeronIds={config.folders.flatMap((item) => (item.meron_folder_id ? [item.meron_folder_id] : []))}
                onPickMeron={addMeronFolder}
              />
            }
          />
        )}
      </SettingsGroup>

      <SettingsGroup title={t('aiSort.sortSection')}>
        <div className="flex min-h-11 items-center justify-between gap-3 px-3.5 py-2">
          <span className="min-w-0 truncate text-xs text-primary">
            {running
              ? t('aiSort.sortInboxLeft', { count: Math.max(0, running.total - running.done) })
              : t('aiSort.sortInbox')}
          </span>
          {running ? (
            <Button variant="secondary" size="sm" leftIcon={Square} onClick={() => void stop()}>
              {t('aiSort.stop')}
            </Button>
          ) : (
            <Button
              variant="secondary"
              size="sm"
              leftIcon={Play}
              disabled={!ready || starting}
              onClick={() => void sortInbox()}
            >
              {t('aiSort.sortInboxAction')}
            </Button>
          )}
        </div>
      </SettingsGroup>

      {/* The history has a view of its own; here, just the way in. */}
      <button
        type="button"
        onClick={() => {
          setLogShown(LOG_PAGE)
          setLogOpen(true)
        }}
        className="flex min-h-11 w-full cursor-pointer items-center justify-between gap-3 rounded-2xl border border-border/60 bg-raised/80 px-3.5 py-2 text-left text-xs text-primary shadow-sm shadow-black/[0.03] hover:bg-hover dark:shadow-black/10"
      >
        <span className="truncate font-medium">{t('aiSort.log')}</span>
        <span className="flex shrink-0 items-center gap-1.5 text-secondary">
          {status.log.length > 0 && status.log.length}
          <ChevronRight size={14} />
        </span>
      </button>
    </div>
  )
}

function ModelFacts({ model, className }: { model: AiModel; className: string }) {
  const { t } = useTranslation()
  const cost = costPer1000(model)
  return (
    <p className={className}>
      {cost !== null && t('aiSort.modelPrice', { cost: formatUsd(cost) })}
      {cost !== null && model.decision && ' · '}
      {model.decision && t('aiSort.modelDecision')}
      {(cost !== null || model.decision) && model.reasoning && ' · '}
      {model.reasoning && <span className="text-amber-600 dark:text-amber-400">{t('aiSort.modelSlow')}</span>}
    </p>
  )
}

// A button like the folder picker's that opens a searchable list of the
// service's models, each with what 1,000 emails would cost.
function ModelPicker({
  value,
  label,
  models,
  failed,
  recent,
  onOpen,
  onPick,
}: {
  value: string
  label: string
  models: AiModel[] | undefined
  failed: boolean
  /** The last few models picked on this service, newest first. */
  recent: string[]
  onOpen: () => void
  onPick: (id: string) => void
}) {
  const { t } = useTranslation()
  const [position, setPosition] = useState<{ x: number; y: number } | null>(null)
  const [query, setQuery] = useState('')
  const buttonRef = useRef<HTMLButtonElement | null>(null)

  const close = () => {
    setPosition(null)
    setQuery('')
  }

  const open = () => {
    const rect = buttonRef.current?.getBoundingClientRect()
    setPosition({ x: rect?.left ?? 0, y: (rect?.bottom ?? 0) + 4 })
    onOpen()
  }

  const words = query.trim().toLowerCase()
  const shown = (models ?? [])
    .filter((model) => !words || `${model.id} ${model.name}`.toLowerCase().includes(words))
    .slice(0, MODELS_SHOWN)
  // The last few picked, on top while not searching.
  const recentShown = words
    ? []
    : recent.map(
        (id) =>
          (models ?? []).find((model) => model.id === id) ?? {
            id,
            name: id,
            price_in_per_m: null,
            price_out_per_m: null,
            structured: false,
            reasoning: false,
          },
      )

  const item = (model: AiModel, key: string) => (
    <MenuItem
      key={key}
      pressed={model.id === value}
      onClick={() => {
        close()
        onPick(model.id)
      }}
      label={
        <span className="flex min-w-0 flex-col text-left">
          <span className="truncate">{model.name || model.id}</span>
          <ModelFacts model={model} className="truncate text-[0.65625rem] font-normal text-secondary" />
        </span>
      }
    />
  )

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        aria-label={t('aiSort.model')}
        aria-expanded={position !== null}
        onClick={() => (position ? close() : open())}
        className="flex w-44 cursor-pointer items-center justify-between gap-2 rounded-xl bg-app py-1.5 pr-2.5 pl-3 text-left text-xs font-semibold text-primary ring-1 ring-border"
      >
        <span className="min-w-0 truncate" title={value || label}>
          {label}
        </span>
        <ChevronDown size={12} className="shrink-0 text-secondary" />
      </button>
      {position && (
        <FloatingContextMenu
          x={position.x}
          y={position.y}
          onClose={close}
          overlay
          overlayClassName="fixed inset-0 z-[61]"
          className={`${flyoutClass} w-[320px]`}
        >
          <div className="px-2 pt-1 pb-1.5">
            <TextInput
              autoFocus
              value={query}
              placeholder={t('aiSort.modelSearch')}
              aria-label={t('aiSort.modelSearch')}
              onChange={(event) => setQuery(event.target.value)}
              className="w-full"
            />
          </div>
          {!models && (
            <div className="px-3 py-2 text-xs font-semibold text-secondary">
              {failed ? t('aiSort.modelsFailed') : t('common.loading')}
            </div>
          )}
          {recentShown.length > 0 && (
            <>
              <p className="px-3 pt-1.5 pb-0.5 text-[0.65625rem] font-semibold text-secondary">{t('aiSort.recentModels')}</p>
              {recentShown.map((model) => item(model, `recent:${model.id}`))}
              <p className="px-3 pt-2 pb-0.5 text-[0.65625rem] font-semibold text-secondary">{t('aiSort.allModels')}</p>
            </>
          )}
          {shown.map((model) => item(model, model.id))}
        </FloatingContextMenu>
      )}
    </>
  )
}
