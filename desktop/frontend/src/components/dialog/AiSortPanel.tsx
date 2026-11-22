// Lets the AI move new mail into the owner's own folders before it
// shows in the Inbox. The key goes to the keychain and never comes
// back; the rest is saved whole, one save at a time, like Rules.

import { LessonList, DecisionLog } from './AiSortHistory'
import { useEffect, useRef, useState } from 'react'
import { ChevronRight, Eye, EyeOff, Play, ShieldCheck, Square, X } from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import {
  aiSortMoveBack,
  checkAiSortKey,
  EMPTY_AI_SORT_CONFIG,
  estimateAiSort,
  forgetAiSortLesson,
  formatUsd,
  loadAiModels,
  loadAiSort,
  loadAiSortLessons,
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
  type AiSortLesson,
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
import { SegmentedRow, SettingRow, SettingsGroup, Switch } from './AccountSettingsRows'
import { RuleFolderPicker, useRuleTargetLabel } from './RuleFolderPicker'
import { ModelPicker, ModelFacts } from './AiModelPicker'

const RUN_POLL_MS = 1500
// History rows shown at first and added by each "View more".
const LOG_PAGE = 50
const MAX_FOLDERS = 25
const MAX_NOTE = 120

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
  // What the reader taught by moving mail by hand.
  const [lessons, setLessons] = useState<AiSortLesson[]>([])
  const [lessonsOpen, setLessonsOpen] = useState(false)
  const [lessonsShown, setLessonsShown] = useState(LOG_PAGE)
  const [forgetting, setForgetting] = useState<number | null>(null)
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

  const refreshLessons = () => void loadAiSortLessons().then(setLessons, () => {})

  useEffect(refreshLessons, [])

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

  const forgetLesson = async (lesson: AiSortLesson) => {
    setForgetting(lesson.id)
    try {
      setLessons(await forgetAiSortLesson(lesson.id))
    } catch {
      showToast(t('aiSort.lessonForgetFailed'), 'error')
      refreshLessons()
    } finally {
      setForgetting(null)
    }
  }

  if (lessonsOpen)
    return (
      <LessonList
        lessons={lessons}
        shown={lessonsShown}
        onMore={() => setLessonsShown(lessonsShown + LOG_PAGE)}
        onBack={() => setLessonsOpen(false)}
        forgetting={forgetting}
        forgetLesson={forgetLesson}
      />
    )
  if (logOpen)
    return (
      <DecisionLog
        entries={status.log}
        shown={logShown}
        onMore={() => setLogShown(logShown + LOG_PAGE)}
        onBack={() => setLogOpen(false)}
        movingBack={movingBack}
        moveBack={moveBack}
      />
    )

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
                  <Button
                    size="sm"
                    disabled={!keyInput.trim() || keyBusy}
                    onClick={() => void saveKey(option, keyInput.trim())}
                  >
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

      {/* What the reader taught by moving mail by hand: a view of its own too. */}
      <button
        type="button"
        onClick={() => {
          setLessonsShown(LOG_PAGE)
          refreshLessons()
          setLessonsOpen(true)
        }}
        className="flex min-h-11 w-full cursor-pointer items-center justify-between gap-3 rounded-2xl border border-border/60 bg-raised/80 px-3.5 py-2 text-left text-xs text-primary shadow-sm shadow-black/[0.03] hover:bg-hover dark:shadow-black/10"
      >
        <span className="truncate font-medium">{t('aiSort.lessons')}</span>
        <span className="flex shrink-0 items-center gap-1.5 text-secondary">
          {lessons.length > 0 && lessons.length}
          <ChevronRight size={14} />
        </span>
      </button>
    </div>
  )
}
