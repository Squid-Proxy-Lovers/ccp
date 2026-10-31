<script lang="ts">
  // Targeted readmission for a client whose grant in this workspace was revoked.
  // Candidates come from the current Clients page.
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Field from '../../lib/ui/Field.svelte';
  import Select from '../../lib/ui/Select.svelte';
  import {app} from '../../lib/app.svelte';
  import ClientAccessForm from './ClientAccessForm.svelte';
  import {readmissionBlocked, type Grant, type Report} from './actions';

  let {grants, onoutcome}: {grants: Grant[]; onoutcome: Report} = $props();
  const candidates = $derived(grants.filter(grant => grant.revoked_at !== null && !grant.client_disabled));
  let choice = $state('');
  let target = $state.raw<Grant | null>(null);

  function start() {
    void app.run(async () => {
      const grant = candidates.find(item => item.id === choice);
      if (!grant || !app.mayLeave()) return;
      if (await readmissionBlocked(app.workspace!.id)) return;
      target = grant;
    }, 'readmit-start');
  }
</script>

{#if target}
  {#key target.id}
    <ClientAccessForm client={target} action="readmit" onclose={() => (target = null)} {onoutcome} />
  {/key}
{:else}
  <Card>
    <CardHeader title="Readmit a client"
      description="Creates a new grant for a registered client after every earlier grant in this workspace was revoked. Revoked, enabled clients from the current Clients page are listed." />
    <CardContent>
      {#if candidates.length}
        <div class="flex max-w-xl flex-wrap items-end gap-3">
          <Field id="readmit-client" label="Revoked client" class="min-w-64 flex-1">
            <Select id="readmit-client" placeholder="Choose a client" bind:value={choice}
              options={candidates.map(grant => ({value: grant.id, label: `${grant.client_name} · ${grant.client_id.slice(0, 8)}`}))} />
          </Field>
          <Button variant="outline" busy={app.working('readmit-start')} disabled={!choice} onclick={start}>Continue</Button>
        </div>
      {:else}
        <p class="text-sm text-muted-foreground">No revoked, enabled clients on the current Clients page. Page through Clients, or open a client there to readmit it.</p>
      {/if}
    </CardContent>
  </Card>
{/if}
