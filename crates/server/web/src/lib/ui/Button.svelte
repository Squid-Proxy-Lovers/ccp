<script lang="ts">
  // Busy buttons keep focus: they are aria-disabled, not natively disabled.
  import type {Snippet} from 'svelte';
  import type {HTMLButtonAttributes} from 'svelte/elements';
  import {LoaderCircle} from '@lucide/svelte';
  import {cn} from './cn';

  type Variant = 'default' | 'secondary' | 'outline' | 'ghost' | 'destructive' | 'link';
  type Size = 'sm' | 'default' | 'lg' | 'icon' | 'icon-sm';
  interface Props extends HTMLButtonAttributes {
    variant?: Variant;
    size?: Size;
    busy?: boolean;
    children?: Snippet;
    ref?: HTMLButtonElement | null;
  }
  let {variant = 'default', size = 'default', busy = false, type = 'button', class: className = '',
    children, onclick, disabled, ref = $bindable(null), ...rest}: Props = $props();

  const variants: Record<Variant, string> = {
    default: 'bg-primary text-primary-foreground shadow-xs hover:bg-primary/90',
    secondary: 'bg-secondary text-secondary-foreground shadow-xs hover:bg-secondary/80',
    outline: 'border bg-background shadow-xs hover:bg-accent hover:text-accent-foreground dark:bg-input/30 dark:border-input dark:hover:bg-input/50',
    ghost: 'hover:bg-accent hover:text-accent-foreground dark:hover:bg-accent/50',
    destructive: 'bg-destructive text-white shadow-xs hover:bg-destructive/90 dark:bg-destructive/60',
    link: 'text-primary underline-offset-4 hover:underline',
  };
  const sizes: Record<Size, string> = {
    sm: 'h-8 gap-1.5 rounded-md px-3 text-[13px]',
    default: 'h-9 gap-2 rounded-md px-4 text-sm',
    lg: 'h-10 gap-2 rounded-md px-6 text-sm',
    icon: 'size-9 rounded-md',
    'icon-sm': 'size-8 rounded-md',
  };
  const inert = $derived(Boolean(disabled) || busy);
  function click(event: MouseEvent & {currentTarget: EventTarget & HTMLButtonElement}) {
    if (inert) { event.preventDefault(); return; }
    onclick?.(event);
  }
</script>

<button
  bind:this={ref}
  {type}
  aria-disabled={inert || undefined}
  aria-busy={busy || undefined}
  class={cn('inline-flex shrink-0 cursor-pointer items-center justify-center whitespace-nowrap font-medium transition-[color,background-color,box-shadow] outline-none select-none focus-visible:ring-2 focus-visible:ring-ring aria-disabled:cursor-not-allowed aria-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*=size-])]:size-4',
    variants[variant], sizes[size], className)}
  onclick={click}
  {...rest}
>
  {#if busy}<LoaderCircle class="animate-spin" aria-hidden="true" />{/if}
  {@render children?.()}
</button>
