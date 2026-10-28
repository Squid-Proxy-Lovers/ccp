<script lang="ts">
  import {Sun, Moon, Monitor, Check} from '@lucide/svelte';
  import Menu from '../lib/ui/Menu.svelte';
  import MenuItem from '../lib/ui/MenuItem.svelte';
  import {theme, type ThemeMode} from '../lib/theme.svelte';
  const modes: {value: ThemeMode; label: string; icon: typeof Sun}[] = [
    {value: 'light', label: 'Light', icon: Sun}, {value: 'dark', label: 'Dark', icon: Moon}, {value: 'system', label: 'System', icon: Monitor},
  ];
</script>
<Menu label="Colour theme" align="end" class="min-w-36"
  triggerClass="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground">
  {#snippet trigger()}
    {#if theme.dark}<Moon class="size-4" />{:else}<Sun class="size-4" />{/if}
  {/snippet}
  {#snippet children(close)}
    {#each modes as mode (mode.value)}
      <MenuItem checked={theme.mode === mode.value} onselect={() => { theme.set(mode.value); close(); }}>
        <mode.icon /> <span class="flex-1">{mode.label}</span>
        {#if theme.mode === mode.value}<Check />{/if}
      </MenuItem>
    {/each}
  {/snippet}
</Menu>
