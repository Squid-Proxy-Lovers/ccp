// SPDX-License-Identifier: AGPL-3.0-or-later
// A navigation decision keeps the receipt in memory; only explicit dismissal
// or an approved identity change clears it.
export async function approveRetainedLink(state, confirm, action) {
  const secret = state.accessSecret;
  if (secret?.kind !== 'enrollment_link') return true;
  const session = state.session, epoch = state.sessionEpoch, locked = state.locked;
  const accepted = await confirm({
    title: `${action} and keep the enrollment link?`,
    body: 'The one-time enrollment link is still shown on this page. Save it privately before leaving or reloading the page. It remains visible until you choose “I\'ve saved the link, dismiss”.',
    confirm: `${action} and keep link`,
  });
  return accepted && state.session === session && state.sessionEpoch === epoch
    && state.locked === locked && state.accessSecret === secret;
}
