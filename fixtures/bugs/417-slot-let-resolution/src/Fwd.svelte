<script lang="ts">import Src from './Src.svelte'; import * as NS from './ns';</script>
<Src let:item><slot name="whole" {item} /></Src>
<Src let:item={it}><slot name="alias" {it} /></Src>
<Src let:item={{ a, b: bb }}><slot name="obj" {a} {bb} /></Src>
<Src let:item={{ a: aa = 2 }}><slot name="dflt" {aa} /></Src>
<Src let:item={{ list: [first] }}><slot name="nested" {first} /></Src>
<Src><div slot="named" let:v><slot name="namedfwd" {v} /></div></Src>
<Src><Src slot="named" let:v={w}><slot name="compchild" {w} /></Src></Src>
<svelte:component this={Src} let:item={sc}><slot name="dyn" {sc} /></svelte:component>
<NS.Src let:item={dot}><slot name="dotted" {dot} /></NS.Src>
{#each [1, 2] as x, i}<slot name="idx" {i} {x} />{/each}
<div><span let:zz><slot name="elem" {zz} /></span></div>
