<script lang="ts">import Src from "./Src.svelte";</script>
<Src let:item={{ a: ra, ...others }}>{ra}{others}</Src>
