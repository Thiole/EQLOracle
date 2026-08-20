import { type ClassValue, clsx } from 'clsx';
import { twMerge } from 'tailwind-merge';

// The standard shadcn-svelte helper -- merges conditional class lists
// (clsx) and resolves conflicting Tailwind utilities in favor of the
// last one given (tailwind-merge), so a component's own default classes
// can be safely overridden by whatever a caller passes in via `class`.
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

// Standard shadcn-svelte helper type: a component's own props, plus an
// optional bindable `ref` to the underlying DOM element it renders.
// Every generated ui/ component imports this.
export type WithElementRef<T, E extends HTMLElement = HTMLElement> = T & { ref?: E | null };

// bits-ui wrapper components (select, scroll-area, ...) re-derive their
// own props from the underlying primitive's props type, minus the
// `child`/`children` snippet props they replace with their own -- these
// three are the standard shadcn-svelte helpers for that.
export type WithoutChild<T> = T extends { child?: any } ? Omit<T, 'child'> : T;
export type WithoutChildren<T> = T extends { children?: any } ? Omit<T, 'children'> : T;
export type WithoutChildrenOrChild<T> = WithoutChildren<WithoutChild<T>>;
