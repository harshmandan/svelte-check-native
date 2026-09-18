import { SvelteComponentTyped } from 'svelte';
export default class Select extends SvelteComponentTyped<
  { loading?: boolean; loadOptions?: any; filterText?: string },
  { select: CustomEvent<any> },
  { item: { item: any } }
> {}
