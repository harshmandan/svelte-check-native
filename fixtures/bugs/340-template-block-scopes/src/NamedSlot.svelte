<script>import Outer from './Outer.svelte'; let { a } = $props(); let item = 0; item = 1;</script>
<Outer let:item><div slot="side">{item}</div></Outer>{a}
