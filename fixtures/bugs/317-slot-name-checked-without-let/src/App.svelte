<script lang="ts">import Card from './Card.svelte';</script>
<Card><div slot="footre">oops</div></Card>
<Card><div slot="footer">ok</div></Card>
<Card><svelte:fragment slot="nope">x</svelte:fragment></Card>
