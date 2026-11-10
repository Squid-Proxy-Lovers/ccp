<script lang="ts">
  import Secret from '../lib/ui/Secret.svelte';
  import {app} from '../lib/app.svelte';
  let {class: className = ''}: {class?: string} = $props();
</script>

{#if app.accessSecret}
  {@const secret = app.accessSecret}
  {@const workspace = app.workspace?.id === secret.workspace_id ? app.workspace : app.workspaces.find(item => item.id === secret.workspace_id)}
  <div class={className}>
    <p class="mb-2 break-words text-sm text-muted-foreground">Workspace <span class="font-medium text-foreground">{workspace?.name || secret.workspace_name || secret.workspace_id}</span></p>
    <Secret title={secret.title} value={secret.value} ondismiss={() => { if (app.accessSecret === secret) app.accessSecret = null; }} />
  </div>
{/if}
