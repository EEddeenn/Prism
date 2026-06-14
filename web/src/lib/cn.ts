import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';

/** Compose class names: clsx flattens/conditionalizes, twMerge resolves
 *  conflicting Tailwind utilities so a caller's `className` always wins over a
 *  component's defaults. Universal recipe: `cn(base, variants, className)`. */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
