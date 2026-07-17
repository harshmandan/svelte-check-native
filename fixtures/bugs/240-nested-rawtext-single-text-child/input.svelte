<script lang="ts">
    const name: string = 'widget';
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
    <script type="application/ld+json">
        { "@context": "https://schema.org", "name": "static" }
    </script>
</svelte:head>

<svg>
    <style></style>
    <style>.dot { fill: red; }</style>
</svg>

<p>{name}</p>
