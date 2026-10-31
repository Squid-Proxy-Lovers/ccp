// SPDX-License-Identifier: AGPL-3.0-or-later
import './app.css';
import {mount} from 'svelte';
import App from './App.svelte';

const target = document.getElementById('app');
if (target) mount(App, {target});
