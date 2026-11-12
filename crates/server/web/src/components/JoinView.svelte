<script lang="ts">
  // Public join page. It shows the recipient how to enroll and never redeems the
  // invitation itself; the client verifies the server and workspace first.
  import {onMount, untrack} from 'svelte';
  import {ShieldCheck} from '@lucide/svelte';
  import Logo from './Logo.svelte';
  import CenteredPage from './CenteredPage.svelte';
  import WorkspaceAvatar from './WorkspaceAvatar.svelte';
  import EnrollmentLinkCard from './EnrollmentLinkCard.svelte';
  import Command from '../lib/ui/Command.svelte';
  import {enrollmentUri, parseEnrollmentLink, watchJoinLinks} from '../lib/enrollment_link.js';
  import {displayLinkName} from '../lib/link_issuance.js';
  let {initialLink, initialError = ''}: {initialLink: ReturnType<typeof parseEnrollmentLink> | null; initialError?: string} = $props();
  let link = $state.raw(untrack(() => initialLink));
  let error = $state(untrack(() => initialError));
  onMount(() => watchJoinLinks(location, history, window, (next, message) => { link = next; error = message; }));
  const host = $derived(link ? new URL(link.https_origin).host : '');
  const anchor = 'rounded font-medium text-foreground underline underline-offset-4 outline-none focus-visible:ring-2 focus-visible:ring-ring';
  const panel = 'w-full max-w-md overflow-hidden rounded-xl border bg-card/90 shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-12px_rgb(0_0_0/0.18)] backdrop-blur-sm dark:shadow-[0_0_0_1px_rgb(255_255_255/0.02),0_16px_40px_-16px_rgb(0_0_0/0.9)]';
  const marker = 'flex size-6 shrink-0 items-center justify-center rounded-md border border-foreground font-mono text-[11px] [&_svg]:size-3.5';
</script>

<svelte:head><title>{link ? `Join ${displayLinkName(link.workspace_name)} · CCP` : 'Join a workspace · CCP'}</title></svelte:head>
<CenteredPage>
  {#if link}
    <header class="rise flex flex-col items-center gap-4 text-center">
      <div class="relative">
        <div class="login-halo pointer-events-none absolute -inset-10 -z-10" aria-hidden="true"></div>
        <WorkspaceAvatar id={link.workspace_id} class="size-14 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" />
        <Logo class="absolute -right-2 -bottom-2 size-6 rounded-[28%] ring-[3px] ring-background" />
      </div>
      <div>
        <h1 class="text-2xl font-semibold tracking-tight break-words">Join <bdi>{displayLinkName(link.workspace_name)}</bdi></h1>
        <p class="mt-1 font-mono text-xs text-muted-foreground">{host}</p>
      </div>
    </header>

    <ol class={['rise rise-2 divide-y', panel]}>
      <li class="grid gap-3 p-5">
        <div class="flex items-center gap-3">
          <span class={marker} aria-hidden="true">01</span>
          <h2 class="text-sm font-semibold">Install the CCP client</h2>
        </div>
        <div class="grid gap-1.5 pl-9">
          <p class="text-sm text-muted-foreground">It installs as <code class="font-mono text-foreground">ccp</code>, or <code class="font-mono text-foreground">ccp-client</code> if that name is taken.</p>
          <p class="flex flex-wrap gap-x-4 gap-y-1 text-sm">
            <a class={anchor} href="https://github.com/Squid-Proxy-Lovers/ccp/releases" target="_blank" rel="noreferrer noopener">Official releases</a>
            <a class={anchor} href="https://github.com/Squid-Proxy-Lovers/ccp/blob/main/docs/workspace-client.md" target="_blank" rel="noreferrer noopener">Setup guide</a>
          </p>
        </div>
      </li>
      <li class="grid gap-3 p-5">
        <div class="flex items-center gap-3">
          <span class={marker} aria-hidden="true">02</span>
          <h2 class="text-sm font-semibold">Run this, then paste your link</h2>
        </div>
        <div class="min-w-0 pl-9">
          {#key enrollmentUri(link)}
            <EnrollmentLinkCard variant="join" value={enrollmentUri(link)} ondismiss={() => (link = null)} />
          {/key}
        </div>
      </li>
      <li class="grid gap-3 p-5">
        <div class="flex items-center gap-3">
          <span class={marker} aria-hidden="true"><ShieldCheck /></span>
          <h2 class="text-sm font-semibold">Your client checks first</h2>
        </div>
        <p class="pl-9 text-sm text-muted-foreground">It verifies this server's certificate and workspace before sending the invitation, then prints how to connect your agents.</p>
      </li>
    </ol>
    <p class="rise rise-3 max-w-md text-center text-xs leading-relaxed text-muted-foreground">
      This page never uses the invitation. The name is a hint from the link; the workspace ID is
      <span class="font-mono break-all">{link.workspace_id}</span>.
    </p>
  {:else}
    <header class="rise flex max-w-sm flex-col items-center gap-4 text-center">
      <div class="relative">
        <div class="login-halo pointer-events-none absolute -inset-10 -z-10" aria-hidden="true"></div>
        <Logo class="size-11 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" />
      </div>
      <h1 class="text-2xl font-semibold tracking-tight">Open your full enrollment link</h1>
    </header>
    <div class={['rise rise-2 grid gap-4 p-5', panel, 'max-w-sm']}>
      {#if error}<p role="alert" class="text-sm text-destructive">{error}</p>{/if}
      <p class="text-sm text-pretty text-muted-foreground">Ask a workspace owner or admin for an enrollment link, then open the complete link. Its secret part was removed from the address bar for privacy, so refreshing won't bring it back.</p>
      <div class="grid gap-1.5 border-t pt-4">
        <p class="text-xs text-muted-foreground">Or run this and paste the link at the hidden prompt:</p>
        <Command command="ccp enroll" />
      </div>
    </div>
  {/if}
</CenteredPage>
