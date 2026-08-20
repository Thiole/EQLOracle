// why: single source of truth for the Debug module's three read-only views
import { writable } from 'svelte/store';
import { api, type DebugEncounterDto, type UnmatchedCoverageDto, type ClassConfigurationsDto } from '../tauri/api';

export const debugEncounters = writable<DebugEncounterDto[] | null>(null);
export const unmatchedCoverage = writable<UnmatchedCoverageDto | null>(null);
export const debugConfigurations = writable<ClassConfigurationsDto | null>(null);

/** why: loaded once on entering Debug; input: none; output: void */
export async function loadDebugModule() {
  const [encounters, coverage, configurations] = await Promise.all([
    api.listDebugEncounters(),
    api.getUnmatchedCoverage(),
    api.getClassConfigurations(),
  ]);
  debugEncounters.set(encounters);
  unmatchedCoverage.set(coverage);
  debugConfigurations.set(configurations);
}
