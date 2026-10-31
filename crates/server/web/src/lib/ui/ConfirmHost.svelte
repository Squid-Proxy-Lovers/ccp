<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Button from './Button.svelte';
  import {confirmState} from './confirm.svelte';
  let open = $state(false);
  let opener: Element | null = null;
  $effect(() => {
    if (confirmState.current && !open) { opener = document.activeElement; open = true; }
  });
  function settle(ok: boolean) {
    const request = confirmState.current;
    confirmState.current = null;
    open = false;
    request?.resolve(ok);
    if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
  }
</script>
<Dialog bind:open title={confirmState.current?.title ?? ''} onclose={() => { if (confirmState.current) settle(false); }}>
  <p class="text-sm leading-relaxed text-muted-foreground">{confirmState.current?.body}</p>
  {#snippet footer()}
    <Button variant="outline" onclick={() => settle(false)}>Cancel</Button>
    <Button variant={confirmState.current?.destructive ? 'destructive' : 'default'} onclick={() => settle(true)}>{confirmState.current?.confirm}</Button>
  {/snippet}
</Dialog>
