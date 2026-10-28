<script lang="ts">
  // An entry write without a confirmed response. Only an exact retry of the
  // same operation is offered; a replacement write is never created here.
  import {Copy, RotateCcw, Trash2} from '@lucide/svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Textarea from '../../lib/ui/Textarea.svelte';
  import {app} from '../../lib/app.svelte';
  import {copyText} from '../../lib/notify.svelte';
  import type {HeldIntent} from './content';

  let {intent, error, onretry, onforget, id}: {
    intent: HeldIntent; error: string; onretry: () => void; onforget: () => void; id: string;
  } = $props();
</script>

<Alert tone="warning" title="Write not confirmed" live>
  <div class="space-y-3">
    <p>{error}</p>
    <p>Don't create a replacement write. Check this exact operation in the same session, or copy it for investigation. Forgetting it locally does not cancel a request that was already sent.</p>
    <Field id={`${id}-operation`} label="Original operation">
      <Textarea id={`${id}-operation`} readonly value={intent.body} rows={5} class="font-mono text-xs text-foreground" />
    </Field>
    <div class="flex flex-wrap gap-2">
      <Button size="sm" busy={app.working(`${id}-retry`)} onclick={onretry}><RotateCcw /> Check or retry exact write</Button>
      <Button size="sm" variant="outline" onclick={() => copyText(intent.body)}><Copy /> Copy original request</Button>
      <Button size="sm" variant="ghost" onclick={onforget}><Trash2 /> Forget local attempt</Button>
    </div>
  </div>
</Alert>
