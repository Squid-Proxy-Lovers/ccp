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
  import Select from '../../lib/ui/Select.svelte';
  import {app as panelApp} from '../../lib/app.svelte';
  import {formFrozen} from '../../lib/identitySwitch.js';
  import {mountedView} from '../../lib/viewLifecycle.js';
  import {LinkAttempt} from '../../lib/link_issuance.js';
  import LinkOptions from './LinkOptions.svelte';
  import {changeLink, linkFields, FormDraft, roles, type Enrollment, type Report} from './actions';

  const app = mountedView(panelApp);
  const {notice, confirmAction} = app;

  let {enrollment, onclose, onoutcome}: {enrollment: Enrollment; onclose: () => void; onoutcome: Report} = $props();
  const form = new FormDraft(`initial-group:${app.workspace?.id}`, {role: 'read', mode: 'short'});
  onMount(() => document.getElementById('group-role')?.focus());

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      const submitted = form;
      if (app.pending) throw new Error('Resolve the original group action first.');
      const workspace = app.workspace!;
      const attempt = new LinkAttempt(app.session, workspace, submitted.get('role'), linkFields(submitted), enrollment, submitted.get('consent') === 'yes');
      await changeLink(app, onoutcome, attempt, () => submitted.drop(), () => confirmAction({
        title: 'Open the single initial group?', confirm: 'Open initial group',
        body: `Open the single initial group in ${workspace.name}? ${attempt.summary()} It cannot reopen once closed. Copy the private link before leaving.`,
      }));
    }, 'initial-group-submit');
  }

  const frozen = $derived(formFrozen(app.busy, app.busyKey, 'initial-group-submit'));

  async function discard() {
    if (frozen) return;
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
      <fieldset disabled={frozen} class="contents">
      <Alert tone="warning" title="One initial group per workspace">
        <p>The default lifetime is 30 minutes. Long-lived links also require a positive admission cap. This phase cannot reopen after closure.</p>
        <p>Copy the group link privately. A lost link cannot be retrieved or replaced by reopening the group.</p>
      </Alert>
      <Field id="group-role" label="Group data access">
        <Select id="group-role" options={roles} bind:value={() => form.get('role'), value => form.set('role', value)} />
      </Field>
      <LinkOptions {form} prefix="group" group />
      <label class="flex items-start gap-3 text-sm">
        <input type="checkbox" required class="mt-0.5 size-4 accent-primary"
          bind:checked={() => form.get('consent') === 'yes', value => form.set('consent', value ? 'yes' : '')} />
        <span>I understand this is the workspace’s single initial group window and it cannot be reopened once closed.</span>
      </label>
      <div class="flex flex-wrap gap-2">
        <Button type="submit" busy={app.working('initial-group-submit')}>Create initial group link</Button>
        <Button variant="ghost" onclick={discard}>Discard group draft</Button>
      </div>
      </fieldset>
    </form>
  </CardContent>
</Card>
