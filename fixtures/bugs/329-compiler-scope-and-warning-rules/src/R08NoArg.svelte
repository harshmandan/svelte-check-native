<script lang="ts">let { a } = $props(); let form = $state<{ valid: boolean }>(); const ok = form?.valid;</script>
{ok}{a}
