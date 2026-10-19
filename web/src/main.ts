import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { initApi } from './lib/api';

// In mock builds this pulls in the in-memory backend; otherwise it's a no-op and mock.ts is tree-shaken.
await initApi();
mount(App, { target: document.getElementById('app')! });
