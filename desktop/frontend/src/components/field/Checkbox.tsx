// Wraps a native checkbox with the desktop field styling and forwards its ref and
// input properties to the underlying control.
// Callers include AddColumnDialog.tsx and EventEditorDialog.tsx.

import { forwardRef } from 'react'
import type { InputHTMLAttributes } from 'react'
import { clsx } from '../../lib/utils'

export interface CheckboxProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'type'> {}

export const Checkbox = forwardRef<HTMLInputElement, CheckboxProps>(function Checkbox({ className, ...rest }, ref) {
  return <input ref={ref} type="checkbox" className={clsx('app-checkbox', className)} {...rest} />
})
