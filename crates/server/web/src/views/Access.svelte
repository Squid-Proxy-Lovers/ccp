<script lang="ts">
  // Clients & access. Sections follow the key's authority: every key sees client
  // grants; owner/admin keys of this workspace also manage invitations and the
  // initial group window; only its owner manages administrators and keys.
  // After a write, the outcome replaces the sections until the user reads
  // current state again, so nothing else can be sent while it is unresolved.
  import {onMount} from 'svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Badge from '../lib/ui/Badge.svelte';
  import Tabs from '../lib/ui/Tabs.svelte';
  import {app} from '../lib/app.svelte';
  import {notice} from '../lib/notify.svelte';
  import {confirmAction} from '../lib/ui/confirm.svelte';
  import {canManageInvitations, canManageTeam} from '../lib/model.js';
  import ClientsTab from './access/ClientsTab.svelte';
  import InvitationsTab from './access/InvitationsTab.svelte';
  import EnrollmentTab from './access/EnrollmentTab.svelte';
  import TeamTab from './access/TeamTab.svelte';
  import Outcome from './access/Outcome.svelte';
  import {readEnrollment, type Enrollment, type GrantPage, type Outcome as Result} from './access/actions';

  type Section = 'clients' | 'invitations' | 'enrollment' | 'team';

  const workspace = $derived(app.workspace!);
  const manager = $derived(canManageInvitations(app.session, app.workspace));
  const owner = $derived(canManageTeam(app.session, app.workspace));
  const sections = $derived([
    {value: 'clients' as Section, label: 'Clients'},
    ...(manager ? [{value: 'invitations' as Section, label: 'Invitations'}, {value: 'enrollment' as Section, label: 'Enrollment'}] : []),
    ...(owner ? [{value: 'team' as Section, label: 'Team'}] : []),
  ]);
  let chosen = $state<Section>('clients');
  const section = $derived(sections.some(item => item.value === chosen) ? chosen : 'clients');

  let outcome = $state.raw<Result | null>(null);
  let generation = $state(0);
  let configuring = $state(false);

  let grants = $state.raw<GrantPage | null>(null);
  let grantsAfter = $state<string | null>(null);
  let grantsError = $state('');
  let enrollment = $state.raw<Enrollment | null>(null);
  let enrollmentError = $state('');

  async function readGrants(after: string | null) {
    try {
      grants = await app.request<GrantPage>('/api/v1/admin/workspaces/grants', {workspace_id: workspace.id, after, limit: 32});
      grantsAfter = after;
      grantsError = '';
    } catch (error) {
      if (app.locked) return;
      grantsError = (error as Error).message;
    }
  }

  async function readWindow() {
    if (!manager) { enrollment = null; return; }
    try {
      enrollment = await readEnrollment(workspace.id);
      enrollmentError = '';
    } catch (error) {
      if (app.locked) return;
      enrollment = null;
      enrollmentError = (error as Error).message;
    }
  }

  onMount(() => { void readGrants(null); void readWindow(); });

  /** "Read current …" after an outcome: re-inspect the workspace and remount the sections. */
  async function reread() {
    if (app.pending) {
      notice('Read the current metadata in the pending action, then explicitly forget it before reloading.', 'warning');
      return;
    }
    if (app.dirty && !await confirmAction({title: 'Discard local work?', confirm: 'Discard and read', destructive: true,
      body: 'Discard local security work and read current administrator and key metadata? Forgotten server requests are not cancelled.'})) return;
    await app.inspectWorkspace();
    app.clean();
    outcome = null;
    configuring = false;
    generation += 1;
    await Promise.all([readGrants(grantsAfter), readWindow()]);
  }

  const report = (next: Result) => { outcome = next; };
</script>

<div class="space-y-6">
  <PageHeader title="Clients & access"
    description="Client identities, their grants and how new clients join this workspace. Agent names and recent changes do not prove that a client is online.">
    {#snippet meta()}
      <Badge tone="outline">{app.instanceScope ? 'Instance key' : owner ? 'Workspace owner' : manager ? 'Workspace admin' : 'Workspace key'}</Badge>
      {#if workspace.archived}<Badge tone="warning">Archived · new admission paused</Badge>{/if}
    {/snippet}
  </PageHeader>

  {#if outcome}
    <Outcome bind:outcome={() => outcome as Result, value => (outcome = value)} onreread={reread} />
  {:else}
    {#if sections.length > 1}
      <Tabs label="Access sections" items={sections} bind:value={() => section, value => (chosen = value)} />
    {/if}
    {#key generation}
      <div role={sections.length > 1 ? 'tabpanel' : undefined} aria-label={sections.find(item => item.value === section)?.label} class="space-y-6">
        {#if section === 'clients'}
          <ClientsTab page={grants} after={grantsAfter} error={grantsError} onpage={readGrants} onoutcome={report} />
        {:else if section === 'invitations'}
          <InvitationsTab {enrollment} grants={grants?.grants ?? []} bind:configuring onoutcome={report} />
        {:else if section === 'enrollment'}
          <EnrollmentTab {enrollment} error={enrollmentError} onread={readWindow}
            onconfigure={() => { configuring = true; chosen = 'invitations'; }} onoutcome={report} />
        {:else}
          <TeamTab onoutcome={report} />
        {/if}
      </div>
    {/key}
  {/if}
</div>
