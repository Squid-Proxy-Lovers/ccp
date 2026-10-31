<script lang="ts">
  // One individual invitation for this workspace.
  import Alert from '../../lib/ui/Alert.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import {app} from '../../lib/app.svelte';
  import {notice} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {LinkAttempt} from '../../lib/link_issuance.js';
  import LinkOptions from './LinkOptions.svelte';
  import {changeLink, linkFields, FormDraft, roles, type Report} from './actions';

  let {unused, onoutcome}: {unused: boolean; onoutcome: Report} = $props();
  const form = new FormDraft(`invite:${app.workspace?.id}`, {role: 'read', mode: 'short'});

  function submit(event: SubmitEvent) {
    event.preventDefault();
    void app.run(async () => {
      if (app.pending) throw new Error('Resolve the original invitation action first.');
      const workspace = app.workspace!;
      await changeLink(onoutcome, new LinkAttempt(app.session, workspace, form.get('role'), linkFields(form)));
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
  <CardHeader title="Create a client enrollment link"
    description="One invitation for this workspace. An existing registered client reuses its identity when redeeming it. This does not select the workspace for that client." />
  <CardContent>
    <form class="grid max-w-xl gap-5" onsubmit={submit}>
      {#if unused}
        <Alert tone="warning" title="This closes the unused initial group phase">
          <p>Issuing an individual invitation permanently closes the unused initial group phase, even before any client redeems it. Choose group enrollment first if you need this phase.</p>
        </Alert>
      {/if}
      <Field id="invite-role" label="Workspace data access">
        <Select id="invite-role" options={roles} bind:value={() => form.get('role'), value => form.set('role', value)} />
      </Field>
      <LinkOptions {form} prefix="invite" />
      <div class="flex flex-wrap gap-2">
        <Button type="submit" busy={app.working('invite-submit')}>Create enrollment link</Button>
        {#if form.touched}<Button variant="ghost" onclick={discard}>Discard draft</Button>{/if}
      </div>
    </form>
  </CardContent>
</Card>
