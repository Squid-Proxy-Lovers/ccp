<script lang="ts">
  import Alert from '../../lib/ui/Alert.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import {linkDateIsLong} from '../../lib/link_issuance.js';
  import type {FormDraft} from './actions';
  let {form, prefix, group = false}: {form: FormDraft; prefix: string; group?: boolean} = $props();
  const mode = $derived(form.get('mode') || 'short');
  const long = $derived(mode === 'never' || mode === 'at' && linkDateIsLong(form.get('date')));
  function setLifetime(field: 'mode' | 'date', value: unknown) {
    form.set(field, value);
    form.set('longLived', '');
    if (field === 'mode') form.set('noExpiry', '');
  }
  const lifetimes = [{value: 'short', label: '30 minutes (default)'}, {value: 'at', label: 'Custom expiry date'}, {value: 'never', label: 'No expiry · confirmation required'}];
</script>

<div class="grid gap-5">
  <Field id={prefix + '-lifetime'} label="Link lifetime">
    <Select id={prefix + '-lifetime'} options={lifetimes} bind:value={() => mode, value => setLifetime('mode', value)} />
  </Field>
  {#if mode === 'at'}
    <Field id={prefix + '-date'} label="Expires at (your local time)" hint="Custom dates are supported. Near-seven-day dates also require confirmation to allow for clock differences.">
      <Input id={prefix + '-date'} type="datetime-local" required aria-describedby={prefix + '-date-hint'}
        bind:value={() => form.get('date'), value => setLifetime('date', value)} />
    </Field>
  {/if}
  {#if mode === 'never'}
    <Alert tone="warning" title="No expiry keeps this invitation usable until its cap is reached or it is revoked">
      <p>Use a short deadline whenever possible. A leaked link may be used much later. Revocation stops unused admissions; it does not remove clients already admitted.</p>
    </Alert>
    <label class="flex items-start gap-3 text-sm"><input type="checkbox" required class="mt-0.5 size-4 accent-primary"
      bind:checked={() => form.get('noExpiry') === 'yes', value => form.set('noExpiry', value ? 'yes' : '')} />
      <span>I accept the risk of an invitation with no expiry and will revoke it when no longer needed.</span>
    </label>
  {:else if long}
    <Alert tone="warning" title="This link expires near or beyond seven days"><p>A leaked link remains usable until its deadline, revocation or use cap. The confirmation appears ten minutes early because browser and server clocks can differ. Confirm this longer admission window deliberately.</p></Alert>
    <label class="flex items-start gap-3 text-sm"><input type="checkbox" required class="mt-0.5 size-4 accent-primary"
      bind:checked={() => form.get('longLived') === 'yes', value => form.set('longLived', value ? 'yes' : '')} />
      <span>I understand this invitation may remain usable for more than seven days.</span>
    </label>
  {/if}
  {#if group}
    <Field id={prefix + '-capacity'} label={long ? 'Maximum client admissions (required)' : 'Maximum client admissions (optional)'} hint="Near-seven-day and longer groups require a positive cap. Leaving this empty permits admissions until a shorter deadline.">
      <Input id={prefix + '-capacity'} type="number" min="1" max="4294967295" step="1" required={long} aria-describedby={prefix + '-capacity-hint'}
        bind:value={() => form.get('capacity'), value => form.set('capacity', value)} />
    </Field>
  {:else}<p class="text-sm text-muted-foreground">This link admits one client. Retrying that same enrollment does not use another admission.</p>{/if}
  <details class="space-y-4 rounded-md border p-3">
    <summary class="cursor-pointer rounded text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring">Advanced · public addresses</summary>
    <p class="pt-2 text-sm text-muted-foreground">Leave blank to use this browser’s HTTPS address and the server’s mTLS listen port. Override them for port mappings or a different public hostname. The server certificate must cover each hostname. This client currently needs a direct mTLS endpoint.</p>
    <Field id={prefix + '-https'} label="Public HTTPS origin (optional)" hint="For example: https://ccp.example:8443">
      <Input id={prefix + '-https'} type="url" placeholder="https://ccp.example:8443" aria-describedby={prefix + '-https-hint'}
        bind:value={() => form.get('https'), value => form.set('https', value)} />
    </Field>
    <Field id={prefix + '-mtls'} label="Public mTLS origin (optional)" hint="For example: https://ccp.example:8444">
      <Input id={prefix + '-mtls'} type="url" placeholder="https://ccp.example:8444" aria-describedby={prefix + '-mtls-hint'}
        bind:value={() => form.get('mtls'), value => form.set('mtls', value)} />
    </Field>
  </details>
</div>
