import { type ReactNode, useEffect, useRef, useState } from 'react'

/** Octop 用 antd 的 Popconfirm / Switch；quill 不引新依赖，这里按同样的交互手写等价件。 */

export type CopyOutcome = 'clipboard' | 'fallback' | 'failed'

/** 剪贴板权限提示既不批准也不拒时，writeText 的 promise 会永远挂着；挂起 = 点了没反应，所以必须竞速。 */
const CLIPBOARD_TIMEOUT_MS = 600

const withTimeout = (work: Promise<void>): Promise<boolean> => {
  let timer: ReturnType<typeof setTimeout> | undefined
  const guard = new Promise<boolean>((resolve) => {
    timer = setTimeout(() => resolve(false), CLIPBOARD_TIMEOUT_MS)
  })
  return Promise.race([
    work.then(() => true).catch(() => false),
    guard,
  ]).finally(() => clearTimeout(timer))
}

/**
 * 复制文本。`navigator.clipboard` 只在安全上下文可用，且可能被权限策略拒掉或直接挂起，
 * 所以超时/失败都再走一次 execCommand 兜底。**两条路都不通必须返回 'failed'**——
 * 静默失败会让这个按钮变成「点了没反应」的假按钮。
 */
export async function copyText(value: string): Promise<CopyOutcome> {
  if (navigator.clipboard?.writeText) {
    if (await withTimeout(navigator.clipboard.writeText(value))) return 'clipboard'
  }
  try {
    const holder = document.createElement('textarea')
    holder.value = value
    holder.setAttribute('readonly', '')
    holder.style.position = 'fixed'
    holder.style.opacity = '0'
    document.body.appendChild(holder)
    holder.select()
    const ok = document.execCommand('copy')
    document.body.removeChild(holder)
    return ok ? 'fallback' : 'failed'
  } catch {
    return 'failed'
  }
}

export function Popconfirm({
  className,
  label,
  icon,
  title,
  description,
  okText,
  cancelText,
  dangerOk,
  pending,
  onConfirm,
}: {
  className: string
  label: string
  icon: ReactNode
  title: string
  description: string
  okText: string
  cancelText: string
  dangerOk: boolean
  pending: boolean
  onConfirm: () => void
}): ReactNode {
  const [open, setOpen] = useState(false)
  const wrapRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: MouseEvent): void => {
      if (!wrapRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'Escape') setOpen(false)
    }
    document.addEventListener('mousedown', onPointerDown)
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('mousedown', onPointerDown)
      document.removeEventListener('keydown', onKeyDown)
    }
  }, [open])

  return (
    <div className="experts-popconfirm-wrap" ref={wrapRef}>
      <button
        type="button"
        className={className}
        aria-label={label}
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
      >
        {icon}
      </button>
      {open ? (
        <div className="experts-popconfirm" role="dialog" aria-label={title}>
          <strong className="experts-popconfirm-title">{title}</strong>
          <span className="experts-popconfirm-desc">{description}</span>
          <span className="experts-popconfirm-actions">
            <button
              type="button"
              className={dangerOk ? 'experts-popconfirm-ok is-danger' : 'experts-popconfirm-ok'}
              disabled={pending}
              onClick={() => {
                setOpen(false)
                onConfirm()
              }}
            >
              {okText}
            </button>
            <button type="button" className="experts-popconfirm-cancel" onClick={() => setOpen(false)}>
              {cancelText}
            </button>
          </span>
        </div>
      ) : null}
    </div>
  )
}

export function ToggleSwitch({
  checked,
  label,
  pending,
  onChange,
}: {
  checked: boolean
  label: string
  pending: boolean
  onChange: (next: boolean) => void
}): ReactNode {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={pending}
      className={checked ? 'experts-switch is-on' : 'experts-switch'}
      onClick={() => onChange(!checked)}
    >
      <span className="experts-switch-knob" />
    </button>
  )
}
