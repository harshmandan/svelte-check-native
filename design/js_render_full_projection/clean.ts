// Consumer of the JS overlay — every access resolves through the
// projected exports surface. Must produce ZERO diagnostics.
import ComponentWithGetters from './component.svelte.svn.js';

const comp: ComponentWithGetters = null as any;
const n: number = comp.test();
const f: InstanceType<typeof comp.Foo> = new comp.Foo();
const b: boolean = comp.bar;
void n;
void f;
void b;
