<script lang="ts">
  import {onMount, untrack} from 'svelte';
  import Logo from './Logo.svelte';
  import Appearance from './Appearance.svelte';
  import EnrollmentLinkCard from './EnrollmentLinkCard.svelte';
  import {enrollmentUri, parseEnrollmentLink, watchJoinLinks} from '../lib/enrollment_link.js';
  import {displayLinkName} from '../lib/link_issuance.js';
  let {initialLink, initialError = ''}: {initialLink: ReturnType<typeof parseEnrollmentLink> | null; initialError?: string} = $props();
  let link = $state.raw(untrack(() => initialLink));
  let error = $state(untrack(() => initialError));
  onMount(() => watchJoinLinks(location, history, window, (next, message) => { link = next; error = message; }));
</script>

<svelte:head><title>Join a workspace · CCP</title></svelte:head>
<div class="min-h-svh bg-background text-foreground">
  <header class="flex h-16 items-center justify-between border-b px-6"><Logo class="size-8 shrink-0" /><Appearance /></header>
  <main class="mx-auto grid max-w-2xl gap-7 px-6 py-12">
    {#if link}
      <div class="space-y-3">
        <h1 class="break-words text-3xl font-semibold tracking-tight">Join <bdi>{displayLinkName(link.workspace_name)}</bdi> on CCP</h1>
        <p class="text-muted-foreground">Enroll your client to share context with this workspace. This page does not redeem the invitation. Your client verifies the server and workspace before reporting success.</p>
      </div>
      <dl class="grid gap-3 border-y py-4 text-sm">
        <div class="grid gap-1"><dt class="text-muted-foreground">Server</dt><dd class="break-all font-mono text-xs">{link.https_origin}</dd></div>
        <div class="grid gap-1"><dt class="text-muted-foreground">Workspace ID · name above is an invitation hint</dt><dd class="break-all font-mono text-xs">{link.workspace_id}</dd></div>
      </dl>
      {#key enrollmentUri(link)}
        <EnrollmentLinkCard value={enrollmentUri(link)} ondismiss={() => link = null} />
      {/key}
      <section class="space-y-3" aria-labelledby="join-install">
        <h2 id="join-install" class="text-xl font-semibold tracking-tight">Install the matching CCP client</h2>
        <p class="text-sm text-muted-foreground">Use an official release, or follow the source-build instructions for your operating system. The installer adds the <code class="font-mono">ccp</code> alias when that name is available; otherwise use <code class="font-mono">ccp-client</code>.</p>
        <div class="flex flex-wrap gap-4 text-sm">
          <a class="rounded underline underline-offset-4 outline-none focus-visible:ring-2 focus-visible:ring-ring" href="https://github.com/Squid-Proxy-Lovers/ccp/releases" target="_blank" rel="noreferrer noopener">Official releases</a>
          <a class="rounded underline underline-offset-4 outline-none focus-visible:ring-2 focus-visible:ring-ring" href="https://github.com/Squid-Proxy-Lovers/ccp/blob/main/docs/workspace-client.md" target="_blank" rel="noreferrer noopener">Client setup instructions</a>
        </div>
        <p class="text-sm text-muted-foreground">For unusually long links, use a private file with <code class="font-mono">ccp enroll -</code>. After enrollment, the client prints the profile and MCP setup next step.</p>
      </section>
    {:else}
      <h1 class="text-3xl font-semibold tracking-tight">Open your complete enrollment link</h1>
      {#if error}<p role="alert" class="text-sm text-destructive">{error}</p>{/if}
      <p class="text-muted-foreground">Ask the workspace owner or admin for an enrollment link. Reopen the original complete link to see its instructions. Its private fragment was removed from the address bar; refreshing does not restore it.</p>
      <p class="text-sm text-muted-foreground">You can also run <code class="font-mono">ccp enroll</code> and paste the complete link at its hidden prompt.</p>
    {/if}
  </main>
</div>
