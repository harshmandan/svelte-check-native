export { default as Foo } from './Foo.svelte';
import Foo from './Foo.svelte';
const bad: number = 'x';
export const f: typeof Foo = Foo;
