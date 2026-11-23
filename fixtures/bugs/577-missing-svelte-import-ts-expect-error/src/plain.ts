// @ts-expect-error - suppressed missing svelte import in plain ts
import { nope } from './MissingFromTs.svelte';

export const val = nope;
