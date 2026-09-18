<script lang="ts">export let n = 1;</script>
<slot item={{ a: n, b: 's', list: [n] }} />
<slot name="named" v={n} />
