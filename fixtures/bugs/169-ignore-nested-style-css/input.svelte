<script lang="ts">
    let visible = true;
</script>

<svelte:head>
    <style>
        @keyframes fadeIn {
            from {
                opacity: 0;
            }
            to {
                opacity: 1;
            }
        }
    </style>
</svelte:head>

{#if visible}
    <p>ready</p>
{/if}
