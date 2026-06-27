<script lang="ts">
  let w = 0;            // number — OK against clientWidth (number)
  let nat = 0;          // number — OK against naturalWidth on <img>
  let anyW = 0;         // custom element resolves to any — OK
</script>
<div bind:clientWidth={w}></div>
<img bind:naturalWidth={nat} alt="x" />
<my-el bind:clientWidth={anyW}></my-el>
