<script lang="ts">
  // Mode, base tint and accent. Native radio groups give arrow-key selection.
  import {Sun, Moon, Monitor, Palette, Check} from '@lucide/svelte';
  import Popover from '../lib/ui/Popover.svelte';
  import {theme, type ThemeMode, type Base, type Accent} from '../lib/theme.svelte';

  const modes: {value: ThemeMode; label: string; icon: typeof Sun}[] = [
    {value: 'light', label: 'Light', icon: Sun}, {value: 'dark', label: 'Dark', icon: Moon}, {value: 'system', label: 'System', icon: Monitor},
  ];
  // Swatches show each option's own colour, independent of the current theme.
  const accents: {value: Accent; label: string; swatch: string}[] = [
    {value: 'neutral', label: 'Neutral', swatch: 'bg-[oklch(0.205_0_0)] dark:bg-[oklch(0.922_0_0)]'},
    {value: 'blue', label: 'Blue', swatch: 'bg-[oklch(0.546_0.245_262.881)]'},
    {value: 'violet', label: 'Violet', swatch: 'bg-[oklch(0.541_0.281_293.009)]'},
    {value: 'green', label: 'Green', swatch: 'bg-[oklch(0.527_0.154_150.069)]'},
    {value: 'orange', label: 'Orange', swatch: 'bg-[oklch(0.553_0.195_38.402)]'},
    {value: 'rose', label: 'Rose', swatch: 'bg-[oklch(0.56_0.235_17)]'},
  ];
  const bases: {value: Base; label: string; swatch: string}[] = [
    {value: 'neutral', label: 'Neutral', swatch: 'bg-[oklch(0.556_0_0)]'},
    {value: 'zinc', label: 'Zinc', swatch: 'bg-[oklch(0.552_0.016_285.938)]'},
    {value: 'slate', label: 'Slate', swatch: 'bg-[oklch(0.554_0.046_257.417)]'},
    {value: 'stone', label: 'Stone', swatch: 'bg-[oklch(0.553_0.013_58.071)]'},
  ];
  const legend = 'mb-2 text-xs font-medium text-muted-foreground';
</script>

<Popover label="Appearance" class="w-72"
  triggerClass="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground aria-expanded:bg-accent aria-expanded:text-foreground">
  {#snippet trigger()}<Palette class="size-4" />{/snippet}
  {#snippet children()}
    <div class="space-y-4">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold">Appearance</h2>
        <button type="button" onclick={() => theme.reset()}
          class="cursor-pointer rounded text-xs text-muted-foreground underline-offset-4 outline-none hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring">Reset</button>
      </div>

      <fieldset>
        <legend class={legend}>Mode</legend>
        <div class="grid grid-cols-3 gap-1 rounded-lg bg-muted p-1">
          {#each modes as mode (mode.value)}
            <label class="relative">
              <input type="radio" name="appearance-mode" value={mode.value} checked={theme.mode === mode.value}
                onchange={() => theme.set(mode.value)} class="peer sr-only" />
              <span class="flex cursor-pointer flex-col items-center gap-1 rounded-md py-2 text-xs font-medium text-muted-foreground transition-colors peer-checked:bg-background peer-checked:text-foreground peer-checked:shadow-sm peer-focus-visible:ring-2 peer-focus-visible:ring-ring hover:text-foreground dark:peer-checked:bg-input/40 [&_svg]:size-4">
                <mode.icon aria-hidden="true" />{mode.label}
              </span>
            </label>
          {/each}
        </div>
      </fieldset>

      <fieldset>
        <legend class={legend}>Accent</legend>
        <div class="flex flex-wrap gap-2">
          {#each accents as accent (accent.value)}
            <label class="relative" title={accent.label}>
              <input type="radio" name="appearance-accent" value={accent.value} checked={theme.accent === accent.value}
                onchange={() => theme.setAccent(accent.value)} class="peer sr-only" />
              <span class={['flex size-8 cursor-pointer items-center justify-center rounded-full ring-offset-2 ring-offset-popover transition-shadow peer-checked:ring-2 peer-checked:ring-foreground/70 peer-focus-visible:ring-2 peer-focus-visible:ring-ring [&_svg]:size-4', accent.swatch]}>
                {#if theme.accent === accent.value}<Check class="text-white mix-blend-difference" aria-hidden="true" />{/if}
              </span>
              <span class="sr-only">{accent.label}</span>
            </label>
          {/each}
        </div>
      </fieldset>

      <fieldset>
        <legend class={legend}>Base</legend>
        <div class="grid grid-cols-2 gap-2">
          {#each bases as base (base.value)}
            <label class="relative">
              <input type="radio" name="appearance-base" value={base.value} checked={theme.base === base.value}
                onchange={() => theme.setBase(base.value)} class="peer sr-only" />
              <span class="flex cursor-pointer items-center gap-2 rounded-md border px-2.5 py-2 text-sm transition-colors peer-checked:border-foreground/40 peer-checked:bg-accent peer-focus-visible:ring-2 peer-focus-visible:ring-ring hover:bg-accent/60">
                <span class={['size-3.5 shrink-0 rounded-full', base.swatch]} aria-hidden="true"></span>{base.label}
              </span>
            </label>
          {/each}
        </div>
      </fieldset>
    </div>
  {/snippet}
</Popover>
