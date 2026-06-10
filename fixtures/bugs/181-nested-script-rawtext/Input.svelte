<script lang="ts">
  let title: string = "Hi";
</script>

<svelte:head>
  <script type="application/ld+json">
    {"@context": "https://schema.org", "name": "Acme", "url": "https://acme.test"}
  </script>
</svelte:head>

<h1>{title}</h1>
