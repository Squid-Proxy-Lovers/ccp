// SPDX-License-Identifier: AGPL-3.0-or-later
import './app.css';
import {mount} from 'svelte';
import App from './App.svelte';
import SetupView from './components/SetupView.svelte';
import JoinView from './components/JoinView.svelte';
import {takeJoinLink} from './lib/enrollment_link.js';
import {takeSetupCode} from './lib/setup.js';

const target = document.getElementById('app');
if (target) {
  if (location.pathname === '/join') {
    let initialLink = null, initialError = '';
    try { initialLink = takeJoinLink(location, history); }
    catch (error) { initialError = error instanceof Error ? error.message : 'Invalid enrollment link.'; }
    mount(JoinView, {target, props: {initialLink, initialError}});
  } else if (location.pathname === '/admin/setup') {
    let initialCode = '', initialError = '';
    try { initialCode = takeSetupCode(location, history); }
    catch (error) { initialError = error instanceof Error ? error.message : 'Invalid setup link.'; }
    mount(SetupView, {target, props: {initialCode, initialError}});
  } else mount(App, {target});
}
