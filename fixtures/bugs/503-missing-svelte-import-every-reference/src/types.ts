import { "a-b" as AB } from './StrGone.svelte';
export type R = typeof import('./TypeofGone.svelte');
export type S = import('./ImportTypeGone.svelte').Thing<string>;
import Req = require('./RequireGone.svelte');
export const r = [Req, AB];
