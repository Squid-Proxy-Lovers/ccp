<script lang="ts">
  // The workspace's single initial group invitation. It needs a capacity, a
  // lifetime or both, plus explicit consent, and can never reopen once closed.
  import {onMount} from 'svelte';
  import Alert from '../../lib/ui/Alert.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import {app} from '../../lib/app.svelte';
  import {notice, stamp} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {initialBounds} from '../../lib/model.js';
  import {changeEnrollment, FormDraft, roles, type Enrollment, type Report} from './actions';

  let {enrollment, onclose, onoutcome}: {enrollment: Enrollment; onclose: () => void; onoutcome: Report} = $props();
  const form = new FormDraft(`initial-group:${app.workspace?.id}`, {role: 'read'});
  let capacityInput: HTMLInputElement | null = $state(null);
  onMount(() => capacityInput?.focus());

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (app.pending) throw new Error('Resolve the original group action first.');
      const workspace = app.workspace!;
      const bounds = initialBounds(form.get('capacity'), form.get('hours'));
      const body = {workspace_id: enrollment.workspace_id, role: form.get('role'), ...bounds};
      const consent = form.get('consent') === 'yes';
      const sent = await changeEnrollment(onoutcome, enrollment, 'initial', body, consent, () => confirmAction({
        title: 'Open the single initial group?', confirm: 'Open initial group',
        body: `Open the single initial group in ${workspace.name}? Capacity: ${bounds.max_clients === null ? 'No count limit' : bounds.max_clients}. Deadline: ${bounds.expires_at === null ? 'None' : stamp(bounds.expires_at)}. It cannot reopen once closed.`,
      }));
      if (sent) form.drop();
    }, 'initial-group-submit');
  }

  async function discard() {
    if (!await confirmAction({title: 'Discard draft?', body: 'Discard this local group draft?', confirm: 'Discard draft', destructive: true})) return;
    form.drop();
    onclose();
    notice('Local group draft discarded.');
  }
</script>

<Card>
  <CardHeader title="Configure initial group enrollment" description={`Workspace ${app.workspace?.name} · ${enrollment.workspace_id}`} />
  <CardContent>
    <form class="grid max-w-xl gap-5" onsubmit={submit}>
      <Alert tone="warning" title="One initial group per workspace">
        <p>Choose a positive admission capacity, a positive lifetime, or both. There is no numeric default. This phase cannot reopen after closure.</p>
        <p>Copy the one-time group secret privately. A lost secret cannot be retrieved or replaced by reopening the group.</p>
      </Alert>
      <Field id="group-role" label="Group data access">
        <Select id="group-role" options={roles} bind:value={() => form.get('role'), value => form.set('role', value)} />
      </Field>
      <div class="grid gap-5 sm:grid-cols-2">
        <Field id="group-capacity" label="Maximum client admissions (optional)">
          <Input id="group-capacity" type="number" min="1" max="4294967295" step="1" bind:ref={capacityInput}
            bind:value={() => form.get('capacity'), value => form.set('capacity', value)} />
        </Field>
        <Field id="group-hours" label="Group lifetime in hours (optional)">
          <Input id="group-hours" type="number" min="1" step="1"
            bind:value={() => form.get('hours'), value => form.set('hours', value)} />
        </Field>
      </div>
      <label class="flex items-start gap-3 text-sm">
        <input type="checkbox" required class="mt-0.5 size-4 accent-primary"
          bind:checked={() => form.get('consent') === 'yes', value => form.set('consent', value ? 'yes' : '')} />
        <span>I understand this is the workspace’s single initial group window and it cannot be reopened once closed.</span>
      </label>
      <div class="flex flex-wrap gap-2">
        <Button type="submit" busy={app.working('initial-group-submit')}>Create initial group invitation</Button>
        <Button variant="ghost" onclick={discard}>Discard group draft</Button>
      </div>
    </form>
  </CardContent>
</Card>
