// why: MapViewer's own container relies on Tailwind's h-full/w-full/relative
// utility classes for its sizing -- without this, the container has no
// explicit size, so it falls back to auto-sizing around its canvas child,
// which the canvas's own size (set by MapViewer's resize()) depends on in
// turn. That circularity is what produced an infinite ResizeObserver loop
// the first time this harness ran without it -- a test-harness bug, not a
// MapViewer one, but a real trap for the next isolated-component harness.
import '../../src/app.css';
import { mount } from 'svelte';
import Harness from './MapViewerHarness.svelte';

mount(Harness, { target: document.getElementById('app')! });
