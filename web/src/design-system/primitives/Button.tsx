import {
  forwardRef,
  cloneElement,
  isValidElement,
  type ComponentPropsWithoutRef,
  type ReactElement,
} from 'react';
import { cn } from '@/lib/cn';

type ButtonVariant = 'primary' | 'secondary' | 'quiet' | 'critical';
type ButtonSize = 'sm' | 'md';

const base =
  'inline-flex items-center justify-center gap-1.5 rounded-control border font-medium whitespace-nowrap transition-colors duration-150 disabled:cursor-not-allowed disabled:opacity-50';

const sizes: Record<ButtonSize, string> = {
  sm: 'h-7 px-2 text-data-s',
  md: 'h-8 px-3 text-data',
};

const variants: Record<ButtonVariant, string> = {
  // primary = max emphasis (bone fill, dark text); use sparingly
  primary: 'border-bone bg-bone text-model-black hover:bg-signal-white',
  secondary: 'border-iron bg-charcoal text-bone hover:border-dust',
  quiet: 'border-transparent bg-transparent text-archive hover:bg-charcoal hover:text-bone',
  critical: 'border-bad-bd bg-transparent text-bad hover:bg-bad-bg',
};

/** Class recipe for styling a non-<button> (e.g. an <a> or label) as a button. */
export function buttonClass(variant: ButtonVariant = 'secondary', size: ButtonSize = 'md', className?: string) {
  return cn(base, sizes[size], variants[variant], className);
}

export interface ButtonProps extends ComponentPropsWithoutRef<'button'> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Merge button styling onto an arbitrary single child instead of a <button>. */
  asChild?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ variant = 'secondary', size = 'md', asChild = false, type, className, children, ...props }, ref) => {
    const cls = buttonClass(variant, size, className);
    if (asChild && isValidElement(children)) {
      const child = children as ReactElement<{ className?: string }>;
      return cloneElement(child, { className: cn(cls, child.props.className) });
    }
    return (
      <button ref={ref} type={type ?? 'button'} className={cls} {...props}>
        {children}
      </button>
    );
  },
);
Button.displayName = 'Button';
