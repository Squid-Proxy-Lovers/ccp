<script lang="ts">
  // One individual invitation for this workspace.
  import Alert from '../../lib/ui/Alert.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Input from '../../lib/ui/Input.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import {app} from '../../lib/app.svelte';
  import {notice} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {invitationDeadline} from '../../lib/model.js';
  import {changeAdmission, FormDraft, roles, type Report} from './actions';

  let {unused, onoutcome}: {unused: boolean; onoutcome: Report} = $props();
  const form = new FormDraft(`invite:${app.workspace?.id}`, {role: 'read', hours: '24'});

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (app.pending) throw new Error('Resolve the original invitation action first.');
      const workspace = app.workspace!;
      await changeAdmission(onoutcome, workspace, 'individual',
        {workspace_id: workspace.id, role: form.get('role'), expires_at: invitationDeadline(form.get('hours'))});
      form.drop();
    }, 'invite-submit');
  }

  async function discard() {
    if (!await confirmAction({title: 'Discard draft?', body: 'Discard this local invitation draft?', confirm: 'Discard draft', destructive: true})) return;
    form.drop();
    notice('Local invitation draft discarded.');
  }
</script>

<Card>
  <CardHeader title="Invite a client"
    description="One invitation for this workspace. An existing registered client reuses its identity when redeeming it. This does not select the workspace for that client." />
  <CardContent>
    <form class="grid max-w-xl gap-5" onsubmit={submit}>
      {#if unused}
        <Alert tone="warning" title="This closes the unused initial group phase">
          <p>Issuing an individual invitation permanently closes the unused initial group phase, even before any client redeems it. Choose group enrollment first if you need this phase.</p>
        </Alert>
      {/if}
      <div class="grid gap-5 sm:grid-cols-2">
        <Field id="invite-role" label="Workspace data access">
          <Select id="invite-role" options={roles} bind:value={() => form.get('role'), value => form.set('role', value)} />
        </Field>
        <Field id="invite-hours" label="Invitation lifetime (hours)" hint="1 to 168 whole hours.">
          <Input id="invite-hours" type="number" min="1" max="168" step="1" required aria-describedby="invite-hours-hint"
            bind:value={() => form.get('hours'), value => form.set('hours', value)} />
        </Field>
      </div>
      <div class="flex flex-wrap gap-2">
        <Button type="submit" busy={app.working('invite-submit')}>Create invitation</Button>
        {#if form.touched}<Button variant="ghost" onclick={discard}>Discard draft</Button>{/if}
      </div>
    </form>
  </CardContent>
</Card>
