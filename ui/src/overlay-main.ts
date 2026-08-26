import { mount } from 'svelte';
import './overlay.css';
import OverlayApp from './lib/overlay/OverlayApp.svelte';

const app = mount(OverlayApp, {
  target: document.getElementById('overlay')!,
});

export default app;
