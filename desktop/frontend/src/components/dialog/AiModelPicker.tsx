// Displays model pricing and lets the user choose a sorting model.
// The picker filters the models returned by the selected provider.
// Selection updates the settings panel through its callback.

import { useRef, useState } from 'react'
import { ChevronDown } from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import { costPer1000, formatUsd, type AiModel } from '../../states/aiSort'
import { TextInput } from '../field/Field'
import { FloatingContextMenu } from '../menu/FloatingContextMenu'
import { MenuItem } from '../menu/MenuItem'
import { flyoutClass } from '../threads/ThreadMoveCopyItems'

const MODELS_SHOWN = 100

export function ModelFacts({ model, className }: { model: AiModel; className: string }) {
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
export function ModelPicker({
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
              <p className="px-3 pt-1.5 pb-0.5 text-[0.65625rem] font-semibold text-secondary">
                {t('aiSort.recentModels')}
              </p>
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
