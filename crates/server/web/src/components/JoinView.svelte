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
</script>

<svelte:head><title>{link ? `Join ${displayLinkName(link.workspace_name)} · CCP` : 'Join a workspace · CCP'}</title></svelte:head>
<CenteredPage>
  {#if link}
    <div class="rise flex flex-col items-center gap-5 text-center">
      <div class="relative">
        <div class="login-halo pointer-events-none absolute -inset-10 -z-10" aria-hidden="true"></div>
        <WorkspaceAvatar id={link.workspace_id} class="size-16 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" />
        <Logo class="absolute -right-2.5 -bottom-2.5 size-7 rounded-[28%] ring-[3px] ring-background" />
      </div>
      <div>
        <h1 class="text-2xl font-semibold tracking-tight break-words">Join <bdi>{displayLinkName(link.workspace_name)}</bdi></h1>
        <p class="mt-1.5 font-mono text-xs text-muted-foreground">on {host}</p>
      </div>
    </div>

    <ol class="rise rise-2 w-full max-w-md divide-y rounded-xl border bg-card/90 shadow-[0_1px_2px_rgb(0_0_0/0.04),0_12px_32px_-12px_rgb(0_0_0/0.18)] backdrop-blur-sm dark:shadow-[0_0_0_1px_rgb(255_255_255/0.02),0_16px_40px_-16px_rgb(0_0_0/0.9)]">
      <li class="flex gap-4 p-5">
        <span class="flex size-6 shrink-0 items-center justify-center rounded-full border font-mono text-xs" aria-hidden="true">1</span>
        <div class="grid min-w-0 gap-1.5">
          <h2 class="text-sm font-semibold">Install the CCP client</h2>
          <p class="text-sm text-muted-foreground">It installs as <code class="font-mono text-foreground">ccp</code>, or <code class="font-mono text-foreground">ccp-client</code> if that name is taken.</p>
          <p class="flex flex-wrap gap-x-4 gap-y-1 text-sm">
            <a class={anchor} href="https://github.com/Squid-Proxy-Lovers/ccp/releases" target="_blank" rel="noreferrer noopener">Official releases</a>
            <a class={anchor} href="https://github.com/Squid-Proxy-Lovers/ccp/blob/main/docs/workspace-client.md" target="_blank" rel="noreferrer noopener">Setup guide</a>
          </p>
        </div>
      </li>
      <li class="flex gap-4 p-5">
        <span class="flex size-6 shrink-0 items-center justify-center rounded-full border font-mono text-xs" aria-hidden="true">2</span>
        <div class="grid min-w-0 flex-1 gap-3">
          <h2 class="text-sm font-semibold">Run this, then paste your link</h2>
          {#key enrollmentUri(link)}
            <EnrollmentLinkCard variant="join" value={enrollmentUri(link)} ondismiss={() => (link = null)} />
          {/key}
        </div>
      </li>
      <li class="flex gap-4 p-5">
        <span class="flex size-6 shrink-0 items-center justify-center rounded-full border text-xs" aria-hidden="true"><ShieldCheck class="size-3.5" /></span>
        <div class="grid min-w-0 gap-1">
          <h2 class="text-sm font-semibold">Your client checks first</h2>
          <p class="text-sm text-muted-foreground">It verifies this server's certificate and workspace before sending the invitation, then prints how to connect your agents.</p>
        </div>
      </li>
    </ol>
    <p class="rise rise-3 max-w-md text-center text-xs leading-relaxed text-muted-foreground">
      This page never uses the invitation. The name comes from the link and is only a hint; the workspace is
      <span class="font-mono break-all">{link.workspace_id}</span>. For very long links, save the link in a private
      file and pipe it to <code class="font-mono">ccp enroll -</code>.
    </p>
  {:else}
    <div class="rise flex max-w-sm flex-col items-center text-center">
      <Logo class="mb-6 size-14 drop-shadow-[0_8px_18px_rgb(0_0_0/0.18)]" />
      <h1 class="text-2xl font-semibold tracking-tight">Open your full enrollment link</h1>
      {#if error}<p role="alert" class="mt-3 text-sm text-destructive">{error}</p>{/if}
      <p class="mt-2 text-sm text-pretty text-muted-foreground">Ask a workspace owner or admin for an enrollment link, then open the complete link. For privacy its secret part was removed from the address bar, so refreshing won't bring it back.</p>
      <div class="mt-6 grid w-full gap-1.5 text-left">
        <p class="text-xs text-muted-foreground">Or run this and paste the link at the hidden prompt:</p>
        <Command command="ccp enroll" />
      </div>
    </div>
  {/if}
</CenteredPage>
