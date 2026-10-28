<script lang="ts">
  // The workspace's one initial group phase: last read state and permanent closure.
  import Alert from '../../lib/ui/Alert.svelte';
  import Badge from '../../lib/ui/Badge.svelte';
  import Button from '../../lib/ui/Button.svelte';
  import Card from '../../lib/ui/Card.svelte';
  import CardContent from '../../lib/ui/CardContent.svelte';
  import CardHeader from '../../lib/ui/CardHeader.svelte';
  import Skeleton from '../../lib/ui/Skeleton.svelte';
  import Delayed from '../../lib/ui/Delayed.svelte';
  import {app} from '../../lib/app.svelte';
  import {stamp} from '../../lib/notify.svelte';
  import {confirmAction} from '../../lib/ui/confirm.svelte';
  import {changeEnrollment, type Enrollment, type Report} from './actions';

  let {enrollment, error, onread, onconfigure, onoutcome}: {
    enrollment: Enrollment | null; error: string; onread: () => Promise<void>; onconfigure: () => void; onoutcome: Report;
  } = $props();

  const phaseTone = {unused: 'outline', open: 'success', closed: 'neutral'} as const;

  function close(current: Enrollment) {
    void app.run(async () => {
      if (!app.mayLeave()) return;
      const name = app.workspace!.name;
      await changeEnrollment(onoutcome, current, 'close', {workspace_id: current.workspace_id}, true, () => confirmAction({
        title: 'Close initial enrollment permanently?', confirm: 'Close permanently', destructive: true,
        body: `Permanently close initial enrollment in ${name}? It cannot reopen. Existing client grants and individual invitations are preserved.`,
      }));
    }, 'enrollment-close');
  }
</script>

<Card>
  <CardHeader title="Initial group enrollment"
    description="This workspace has one initial group phase. Closing it is terminal; existing client grants and individual invitations are preserved. Current observations are not an opening permission lease."
    action={readAction} />
  <CardContent class="space-y-4">
    {#if error}
      <Alert tone="destructive" title="Initial enrollment state unavailable">
        <p>{error}</p>
        <p>No unused, open or closed state was assumed. Read the current state again before an initial-group action.</p>
      </Alert>
    {:else if !enrollment}
      <Delayed>
        <div class="grid gap-3 sm:grid-cols-[10rem_1fr]" role="status" aria-busy="true">
          <span class="sr-only">Loading enrollment state</span>
          {#each ['w-24', 'w-48', 'w-36'] as width, index (index)}<Skeleton class="h-3.5 w-20 opacity-70" /><Skeleton class={['h-3.5', width]} />{/each}
        </div>
      </Delayed>
    {:else}
      {@const current = enrollment}
      <div class="flex flex-wrap items-center gap-2 text-sm">
        <span class="text-muted-foreground">Last read window</span>
        <Badge tone={phaseTone[current.phase]}>{current.phase === 'unused' ? 'Unused' : current.phase === 'open' ? 'Open' : 'Closed'}</Badge>
        {#if current.closed_at !== null}<span class="text-muted-foreground">Closed {stamp(current.closed_at)}</span>{/if}
      </div>
      {#if current.invitation}
        <dl class="grid gap-x-6 gap-y-2 text-sm sm:grid-cols-[auto_1fr]">
          <dt class="text-muted-foreground">Group invitation</dt><dd class="font-mono text-xs">{current.invitation.invitation_id}</dd>
          <dt class="text-muted-foreground">Admissions</dt>
          <dd class="tabular">{current.invitation.uses}{current.invitation.max_uses === null ? ' · No count limit' : ' / ' + current.invitation.max_uses}</dd>
          <dt class="text-muted-foreground">Deadline</dt>
          <dd>{current.invitation.expires_at === null ? 'No deadline' : stamp(current.invitation.expires_at)}</dd>
        </dl>
      {:else if current.phase === 'closed'}
        <p class="text-sm text-muted-foreground">The initial window was closed without a group invitation.</p>
      {/if}
      {#if current.phase === 'unused'}
        <Alert tone="warning" title="Individual invitations close this phase">
          <p>Issuing an individual invitation permanently closes this unused initial group phase, even before any client redeems it. Choose group enrollment first if you need this phase.</p>
        </Alert>
      {/if}
      {#if current.phase === 'unused' && current.has_admission_history}
        <p class="text-sm text-muted-foreground">Client admission history exists, including revoked grants. A new initial group phase cannot be opened.</p>
      {/if}
      {#if current.workspace_archived}
        <p class="text-sm text-muted-foreground">Archived: new group issuance is paused. Authorized security closure remains available.</p>
      {/if}
      {#if current.can_open || current.phase !== 'closed'}
        <div class="flex flex-wrap gap-2 border-t pt-4">
          {#if current.can_open}<Button variant="outline" onclick={onconfigure}>Configure initial group</Button>{/if}
          {#if current.phase !== 'closed'}
            <Button variant="destructive" busy={app.working('enrollment-close')} onclick={() => close(current)}>Close initial enrollment permanently</Button>
          {/if}
        </div>
      {/if}
    {/if}
  </CardContent>
</Card>

{#snippet readAction()}
  <Button variant="outline" size="sm" busy={app.working('enrollment-read')} onclick={() => app.run(onread, 'enrollment-read')}>Read current state</Button>
{/snippet}
