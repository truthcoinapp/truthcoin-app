// Markets made in this session that may not be in a block yet: their page waits for them instead of saying "no such
// market", however the user gets back to it.
import { writable } from "svelte/store";

export const created = writable<Set<string>>(new Set());

export function noteCreated(id: string) {
  created.update((s) => new Set(s).add(id));
}
