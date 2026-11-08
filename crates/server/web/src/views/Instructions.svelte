<script lang="ts">
  // Master instructions. Global instructions apply to every session; a
  // session's instructions add detail for agents in that session.
  import {onMount} from 'svelte';
  import {RefreshCw} from '@lucide/svelte';
  import PageHeader from '../components/PageHeader.svelte';
  import Alert from '../lib/ui/Alert.svelte';
  import Button from '../lib/ui/Button.svelte';
  import Skeleton from '../lib/ui/Skeleton.svelte';
  import Delayed from '../lib/ui/Delayed.svelte';
  import SkeletonCard from '../lib/ui/skeleton/SkeletonCard.svelte';
  import Board from './instructions/Board.svelte';
  import {app} from '../lib/app.svelte';
  import {enc} from '../lib/api';
  import type {Instructions} from '../lib/types';

  let {global = false}: {global?: boolean} = $props();
  // svelte-ignore state_referenced_locally
  const name = global ? null : app.session;
  const GLOBAL = '/v1/admin/master';
  const sessionPath = name ? `/v1/admin/sessions/${enc(name)}/master` : '';

  let globalRecord = $state.raw<Instructions | null>(null);
  let sessionRecord = $state.raw<Instructions | null>(null);
  let failed = $state('');

  async function load() {
    try {
      const [nextGlobal, nextSession] = await Promise.all([
        app.request<Instructions>(GLOBAL),
        name ? app.request<Instructions>(sessionPath) : Promise.resolve(null),
      ]);
      globalRecord = nextGlobal;
      sessionRecord = nextSession;
      failed = '';
    } catch (error) {
      if (!app.locked) failed = (error as Error).message;
    }
  }
  onMount(() => { void load(); });
</script>

<div class="space-y-6">
  <PageHeader title={global ? 'Global instructions' : 'Master instructions'}
    description={global
      ? 'Instructions for every agent on this server, in every session. Session instructions add detail on top.'
      : `Instructions for agents working in ${name}. Global instructions still apply and take precedence.`} />

  <Alert title="Saves go live immediately">
    <p>Agents receive changed instructions with their next CCP tool call (master_instructions, brief_me and most responses). That doesn't prove they read or followed them.</p>
  </Alert>

  {#if failed}
    <Alert tone="destructive" title="Instructions unavailable" live>
      {#snippet action()}
        <Button size="sm" variant="outline" busy={app.working('instructions-load')} onclick={() => app.run(load, 'instructions-load')}><RefreshCw /> Try again</Button>
      {/snippet}
      <p>{failed}</p>
    </Alert>
  {:else if !globalRecord}
    <Delayed>
      <div class={['grid gap-6', !global && 'xl:grid-cols-2']}>
        {#each global ? [0] : [0, 1] as index (index)}
          <SkeletonCard action label={index === 0 ? 'Loading instructions' : undefined}>
            <div class="space-y-3 px-5"><Skeleton class="h-64 w-full" /><Skeleton class="h-9 w-40" /></div>
          </SkeletonCard>
        {/each}
      </div>
    </Delayed>
  {:else}
    <div class={['grid items-start gap-6', !global && 'xl:grid-cols-2']}>
      {#if name && sessionRecord}
        <Board title="Session instructions" description={`Applies to agents in ${name}.`} path={sessionPath} record={sessionRecord} />
        <Board title="Global instructions" description="Applies to every session. Edit under Global instructions." path={GLOBAL} record={globalRecord} editable={false} />
      {:else}
        <Board title="Global instructions" description="Applies to every agent on this server." path={GLOBAL} record={globalRecord}
          onsaved={next => (app.globalMaster = next)} />
      {/if}
    </div>
  {/if}
</div>
