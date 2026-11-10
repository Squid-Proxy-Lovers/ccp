<script lang="ts">
  import Secret from '../lib/ui/Secret.svelte';
  import {app} from '../lib/app.svelte';
  let {class: className = ''}: {class?: string} = $props();
</script>

{#if app.accessSecret}
  {@const secret = app.accessSecret}
  <div class={className}>
    <Secret title={secret.title} value={secret.value} ondismiss={() => { if (app.accessSecret === secret) app.accessSecret = null; }} />
  </div>
{/if}
