// why: which module the sidebar/main area shows -- a store, not App.svelte
// local state, so any component (Game Data's "open in Combat →") can
// request a module switch without a callback threaded down through
// however many layers separate it from App.svelte.
import { writable } from 'svelte/store';

export const activeModule = writable('combat');
