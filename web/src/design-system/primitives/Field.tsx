import {
  forwardRef,
  cloneElement,
  isValidElement,
  type ComponentPropsWithoutRef,
  type ReactElement,
  type ReactNode,
} from 'react';
import { Search } from 'lucide-react';
import { cn } from '@/lib/cn';

/* Field / TextInput / SearchField — accessible form primitives. `Field` owns the
 * label↔control↔help↔error wiring: it renders a <label htmlFor>, optional help
 * and error text, and injects a composed `aria-describedby` onto its single
 * child control (clone-injected, guarded by isValidElement) so screen readers
 * announce the help/error in context. Error text is tone `bad`. */

const inputBox =
  'rounded-control border border-iron bg-charcoal px-2 py-1 text-data text-bone placeholder:text-dust';

export interface FieldProps {
  id: string;
  label: ReactNode;
  required?: boolean;
  help?: ReactNode;
  error?: ReactNode;
  /** The single control to wire (input/select/textarea). */
  children: ReactNode;
  className?: string;
}

/** Labelled form field: wires <label htmlFor>, help + error ids, and injects a
 *  composed aria-describedby onto the child control. */
export function Field({ id, label, required = false, help, error, children, className }: FieldProps) {
  const helpId = help ? `${id}-help` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [helpId, errorId].filter(Boolean).join(' ') || undefined;

  // Clone-inject the wiring onto the child control (guarded). Caller-supplied
  // id / aria-describedby / aria-invalid still win via the child's own props.
  const control = isValidElement(children)
    ? cloneElement(children as ReactElement<Record<string, unknown>>, {
        id: (children as ReactElement<{ id?: string }>).props.id ?? id,
        'aria-describedby':
          (children as ReactElement<{ ['aria-describedby']?: string }>).props['aria-describedby'] ?? describedBy,
        'aria-invalid':
          (children as ReactElement<{ ['aria-invalid']?: boolean }>).props['aria-invalid'] ?? (error ? true : undefined),
      })
    : children;

  return (
    <div className={cn('flex flex-col gap-1', className)}>
      <label htmlFor={id} className="metadata-label">
        {label}
        {required && (
          <span aria-hidden className="ml-0.5 text-bad">
            *
          </span>
        )}
      </label>
      {control}
      {help && (
        <p id={helpId} className="text-data-s text-archive">
          {help}
        </p>
      )}
      {error && (
        <p id={errorId} className="text-data-s text-bad">
          {error}
        </p>
      )}
    </div>
  );
}

export type TextInputProps = ComponentPropsWithoutRef<'input'>;

/** Thin styled native text input — charcoal box, iron hairline, mono-faint placeholder. */
export const TextInput = forwardRef<HTMLInputElement, TextInputProps>(({ type, className, ...props }, ref) => (
  <input ref={ref} type={type ?? 'text'} className={cn(inputBox, className)} {...props} />
));
TextInput.displayName = 'TextInput';

export type SearchFieldProps = ComponentPropsWithoutRef<'input'>;

/** Compact search input — same styled box with a leading lucide Search icon. */
export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(
  ({ type, className, ...props }, ref) => (
    <div className="relative inline-flex items-center">
      <Search aria-hidden strokeWidth={1.8} className="pointer-events-none absolute left-2 size-3.5 text-dust" />
      <input
        ref={ref}
        type={type ?? 'search'}
        className={cn(inputBox, 'w-full pl-7', className)}
        {...props}
      />
    </div>
  ),
);
SearchField.displayName = 'SearchField';
