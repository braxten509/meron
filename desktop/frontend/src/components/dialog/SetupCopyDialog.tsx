import { useEffect, useRef, useState } from 'react'
import {
  ArrowLeftRight,
  ArrowRight,
  ArrowLeft,
  CheckCircle2,
  Monitor,
  RefreshCw,
  Smartphone,
  TriangleAlert,
  X,
  type LucideIcon,
} from 'lucide-react'
import { useTranslation } from '../../lib/i18n'
import { Dialog } from './Dialog'
import { Button } from '../button/Button'
import { IconButton } from '../button/IconButton'
import { clsx } from '../../lib/utils'
import {
  cancelSetupCopy,
  reloadAfterReplace,
  runSetupCopy,
  setSetupKeep,
  setupCopyStatus,
  startSetupCopy,
  type SetupCopyStatus,
  type SetupCounts,
  type SetupDevice,
} from '../../states/setupCopy'

/** How often the screen asks the core where the copy is. */
const POLL_MS = 500

/** The error codes the core reports that have their own message. */
const KNOWN_ERRORS = new Set([
  'no_network',
  'expired',
  'too_many_attempts',
  'wrong_code',
  'refused',
  'other_network',
  'unreachable',
  'peer_left',
  'peer_failed',
  'peer_bad_copy',
  'bad_copy',
  'safety_failed',
  'apply_failed',
  'collect_failed',
])

export function setupCopyErrorKey(code: string): string {
  return KNOWN_ERRORS.has(code) ? `setupCopy.error.${code}` : 'setupCopy.error.broken'
}

/**
 * "Copy setup to another device", the computer's side: show a QR code (and the
 * address and code to type), then the shared Keep/Erased screen with a swap
 * button and Copy. The core does the networking; this polls its status.
 */
export function SetupCopyDialog({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation()
  const [status, setStatus] = useState<SetupCopyStatus | null>(null)
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)
  const reloaded = useRef(false)

  const start = async () => {
    setConfirming(false)
    reloaded.current = false
    try {
      setStatus(await startSetupCopy())
    } catch {
      setStatus(null)
    }
  }

  useEffect(() => {
    void start()
    const timer = window.setInterval(() => {
      setupCopyStatus()
        .then(setStatus)
        .catch(() => {})
    }, POLL_MS)
    return () => {
      window.clearInterval(timer)
      void cancelSetupCopy().catch(() => {})
    }
  }, [])

  // This computer's setup was replaced: everything loaded is stale.
  useEffect(() => {
    if (status?.stage === 'done' && status.replaced && !reloaded.current) {
      reloaded.current = true
      void reloadAfterReplace()
    }
  }, [status?.stage, status?.replaced])

  const stage = status?.stage ?? 'idle'
  const busyStage = stage === 'copying' || stage === 'applying'

  const close = () => {
    if (busyStage) return
    onClose()
  }

  return (
    <Dialog
      onClose={close}
      dismissible={!busyStage}
      closeOnBackdrop={false}
      aria-label={t('setupCopy.title')}
      className="max-w-lg p-6 flex flex-col gap-5"
    >
      <div className="flex items-start justify-between gap-4">
        <div className="flex items-center gap-3 min-w-0">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-2xl bg-accent/10 text-accent">
            <ArrowLeftRight size={17} />
          </div>
          <div className="min-w-0">
            <h2 className="text-[0.9375rem] font-bold tracking-tight leading-tight">{t('setupCopy.title')}</h2>
            <p className="text-[0.65625rem] text-secondary mt-1 font-medium">{t('setupCopy.subtitle')}</p>
          </div>
        </div>
        <IconButton icon={X} iconSize={15} label={t('buttons.close')} size="sm" onClick={close} disabled={busyStage} />
      </div>

      {(stage === 'idle' || stage === 'waiting') && <WaitingView status={status} />}

      {(stage === 'connected' || stage === 'copying' || stage === 'applying') && status && (
        <DirectionView
          status={status}
          confirming={confirming}
          busy={busy}
          onSwap={async () => {
            const other: SetupDevice = status.keep === 'phone' ? 'computer' : 'phone'
            setStatus(await setSetupKeep(other))
          }}
          onCopy={() => setConfirming(true)}
          onCancelConfirm={() => setConfirming(false)}
          onConfirm={async () => {
            setBusy(true)
            try {
              setStatus(await runSetupCopy())
            } finally {
              setBusy(false)
              setConfirming(false)
            }
          }}
        />
      )}

      {stage === 'done' && status && <DoneView status={status} />}

      {stage === 'failed' && status && (
        <div className="flex items-start gap-2.5 rounded-2xl bg-rose-50 dark:bg-rose-950/20 px-4 py-3">
          <TriangleAlert size={15} className="mt-0.5 shrink-0 text-rose-500" />
          <p className="text-[0.75rem] leading-relaxed font-medium text-rose-600 dark:text-rose-400">
            {t(setupCopyErrorKey(status.error))}
          </p>
        </div>
      )}

      <div className="flex justify-end gap-2 select-none">
        {stage === 'failed' && (
          <Button variant="secondary" onClick={() => void start()}>
            {t('setupCopy.tryAgain')}
          </Button>
        )}
        <Button variant={stage === 'done' ? 'primary' : 'ghost'} onClick={close} disabled={busyStage}>
          {stage === 'done' ? t('buttons.done') : t('buttons.cancel')}
        </Button>
      </div>
    </Dialog>
  )
}

function WaitingView({ status }: { status: SetupCopyStatus | null }) {
  const { t } = useTranslation()
  const [now, setNow] = useState(() => Date.now() / 1000)
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now() / 1000), 1000)
    return () => window.clearInterval(timer)
  }, [])
  if (!status || status.stage !== 'waiting') {
    return (
      <div className="flex items-center justify-center py-16 text-secondary">
        <RefreshCw size={16} className="animate-spin" />
      </div>
    )
  }
  // The core's clock and ours agree here; it runs on this computer.
  const left = Math.max(0, Math.round(status.expires_at - now))
  const time = `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`
  return (
    <div className="flex flex-col items-center gap-4">
      <p className="text-[0.75rem] text-secondary leading-relaxed font-medium text-center px-2">
        {t('setupCopy.scanHint')}
      </p>
      {/* The core draws the QR code; black on white so any camera reads it. */}
      <div
        className="rounded-2xl bg-white p-2 shadow-sm [&_svg]:block [&_svg]:h-[200px] [&_svg]:w-[200px]"
        dangerouslySetInnerHTML={{ __html: status.qr_svg }}
      />
      <div className="w-full rounded-2xl bg-hover px-4 py-3 flex flex-col gap-2">
        <p className="text-[0.65625rem] text-secondary font-semibold">{t('setupCopy.typeHint')}</p>
        <div className="flex items-center justify-between gap-3">
          <span className="text-[0.6875rem] text-secondary font-medium">{t('setupCopy.address')}</span>
          <span className="font-mono text-[0.875rem] font-bold select-all">{status.address}</span>
        </div>
        <div className="flex items-center justify-between gap-3">
          <span className="text-[0.6875rem] text-secondary font-medium">{t('setupCopy.code')}</span>
          <span className="font-mono text-[0.875rem] font-bold tracking-widest select-all">{status.code}</span>
        </div>
      </div>
      <p className="text-[0.65625rem] text-secondary font-medium flex items-center gap-1.5">
        <RefreshCw size={11} className="animate-spin" />
        {t('setupCopy.waitingFor', { time })}
      </p>
    </div>
  )
}

function countsLine(t: (key: string, values?: Record<string, unknown>) => string, counts: SetupCounts | null) {
  const c = counts ?? { accounts: 0, folders: 0, keys: 0 }
  return {
    accounts: t('setupCopy.accounts', { count: c.accounts }),
    folders: t('setupCopy.folders', { count: c.folders }),
    keys: t('setupCopy.keys', { count: c.keys }),
  }
}

function DirectionView({
  status,
  confirming,
  busy,
  onSwap,
  onCopy,
  onCancelConfirm,
  onConfirm,
}: {
  status: SetupCopyStatus
  confirming: boolean
  busy: boolean
  onSwap: () => void
  onCopy: () => void
  onCancelConfirm: () => void
  onConfirm: () => void
}) {
  const { t } = useTranslation()
  const countsOf = (device: SetupDevice) => (status.me === device ? status.mine : status.peer)
  const keep = (status.keep || 'computer') as SetupDevice
  const erased: SetupDevice = keep === 'phone' ? 'computer' : 'phone'
  const erasedCounts = countsLine(t, countsOf(erased))
  const working = status.stage === 'copying' || status.stage === 'applying'
  const locked = working || status.starting || busy

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-stretch gap-2">
        <DeviceBox device="phone" kept={keep === 'phone'} counts={countsOf('phone')} me={status.me === 'phone'} />
        <div className="flex flex-col items-center justify-center gap-2 px-1">
          {keep === 'phone' ? (
            <ArrowRight size={18} className="text-accent" />
          ) : (
            <ArrowLeft size={18} className="text-accent" />
          )}
          <IconButton
            icon={ArrowLeftRight}
            iconSize={14}
            label={t('setupCopy.swap')}
            size="sm"
            onClick={onSwap}
            disabled={locked}
          />
        </div>
        <DeviceBox
          device="computer"
          kept={keep === 'computer'}
          counts={countsOf('computer')}
          me={status.me === 'computer'}
        />
      </div>

      <p className="text-[0.75rem] leading-relaxed font-medium text-center px-2">
        {t(keep === 'phone' ? 'setupCopy.summaryPhoneKept' : 'setupCopy.summaryComputerKept', erasedCounts)}
      </p>

      {working && (
        <p className="flex items-center justify-center gap-2 text-[0.75rem] text-secondary font-medium">
          <RefreshCw size={12} className="animate-spin" />
          {status.stage === 'applying' ? t('setupCopy.applying') : t('setupCopy.copying')}
        </p>
      )}

      {!working && !confirming && (
        <div className="flex justify-center">
          <Button variant="primary" onClick={onCopy} disabled={locked}>
            {status.starting && <RefreshCw size={11} className="animate-spin" />}
            <span>{t('setupCopy.copy')}</span>
          </Button>
        </div>
      )}

      {!working && confirming && (
        <div className="rounded-2xl border border-rose-200 dark:border-rose-900/50 px-4 py-3 flex flex-col gap-3">
          <p className="text-[0.75rem] font-bold">
            {t(erased === 'phone' ? 'setupCopy.confirmErasePhone' : 'setupCopy.confirmEraseComputer')}
          </p>
          <p className="text-[0.6875rem] text-secondary leading-relaxed font-medium">{t('setupCopy.confirmBody')}</p>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm" onClick={onCancelConfirm} disabled={busy}>
              {t('buttons.cancel')}
            </Button>
            <Button variant="danger" size="sm" onClick={onConfirm} disabled={busy}>
              {t('setupCopy.confirmAction')}
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

function DeviceBox({
  device,
  kept,
  counts,
  me,
}: {
  device: SetupDevice
  kept: boolean
  counts: SetupCounts | null
  me: boolean
}) {
  const { t } = useTranslation()
  const Icon: LucideIcon = device === 'phone' ? Smartphone : Monitor
  const line = countsLine(t, counts)
  return (
    <div
      className={clsx(
        'flex-1 min-w-0 rounded-2xl border px-3 py-3 flex flex-col items-center gap-1.5 text-center transition-colors',
        kept ? 'border-accent/40 bg-accent/5' : 'border-border/70 bg-hover/50',
      )}
    >
      <Icon size={22} className={kept ? 'text-accent' : 'text-secondary'} />
      <span className="text-[0.8125rem] font-bold">
        {t(device === 'phone' ? 'setupCopy.phone' : 'setupCopy.computer')}
        {me && <span className="text-secondary font-medium"> · {t('setupCopy.thisDevice')}</span>}
      </span>
      <span
        className={clsx(
          'rounded-full px-2 py-0.5 text-[0.625rem] font-bold',
          kept ? 'bg-accent text-accent-label' : 'bg-rose-50 text-rose-600 dark:bg-rose-950/30 dark:text-rose-400',
        )}
      >
        {kept ? t('setupCopy.keep') : t('setupCopy.erased')}
      </span>
      <span className="text-[0.625rem] text-secondary font-medium leading-snug">
        {line.accounts} · {line.folders} · {line.keys}
      </span>
    </div>
  )
}

function DoneView({ status }: { status: SetupCopyStatus }) {
  const { t } = useTranslation()
  const failures = status.result?.secret_failures ?? 0
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-start gap-2.5 rounded-2xl bg-accent/10 px-4 py-3">
        <CheckCircle2 size={15} className="mt-0.5 shrink-0 text-accent" />
        <p className="text-[0.75rem] leading-relaxed font-medium">
          {status.replaced ? t('setupCopy.doneReceived') : t('setupCopy.doneSent')}
        </p>
      </div>
      {status.replaced && failures > 0 && (
        <p className="text-[0.6875rem] text-rose-500 px-1 font-medium">
          {t('setupCopy.secretFailures', { count: failures })}
        </p>
      )}
    </div>
  )
}
