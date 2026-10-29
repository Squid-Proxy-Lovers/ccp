// SPDX-License-Identifier: AGPL-3.0-or-later
import './app.css';
import {mount} from 'svelte';
import App from './App.svelte';
import SetupView from './components/SetupView.svelte';
import {takeSetupCode} from './lib/setup.js';

const target = document.getElementById('app');
if (target) {
  if (location.pathname === '/admin/setup') {
    let initialCode = '', initialError = '';
    try { initialCode = takeSetupCode(location, history); }
    catch (error) { initialError = error instanceof Error ? error.message : 'Invalid setup link.'; }
    mount(SetupView, {target, props: {initialCode, initialError}});
  } else mount(App, {target});
}
