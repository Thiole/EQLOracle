// why: single source of truth for the Debug module's four read-only views
import { writable } from 'svelte/store';
import {
  api,
  type DebugEncounterDto,
  type UnmatchedCoverageDto,
  type ClassConfigurationsDto,
  type GameStateDto,
} from '../tauri/api';

export const debugEncounters = writable<DebugEncounterDto[] | null>(null);
export const unmatchedCoverage = writable<UnmatchedCoverageDto | null>(null);
export const debugConfigurations = writable<ClassConfigurationsDto | null>(null);
export const gameState = writable<GameStateDto | null>(null);

/** why: loaded once on entering Debug; input: none; output: void */
export async function loadDebugModule() {
  const [encounters, coverage, configurations, state] = await Promise.all([
    api.listDebugEncounters(),
    api.getUnmatchedCoverage(),
    api.getClassConfigurations(),
    api.getGameState(),
  ]);
  debugEncounters.set(encounters);
  unmatchedCoverage.set(coverage);
  debugConfigurations.set(configurations);
  gameState.set(state);
}

/** why: Game State is a live snapshot, worth re-polling without leaving
 * the tab -- unlike the other three panels here, which are fine loaded
 * once. input: none; output: void */
export async function refreshGameState() {
  gameState.set(await api.getGameState());
}
