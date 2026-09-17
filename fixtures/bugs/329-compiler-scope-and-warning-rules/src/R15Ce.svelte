<svelte:options customElement="my-el" />
<script>let { a } = $props();</script>
<slot />{a}
