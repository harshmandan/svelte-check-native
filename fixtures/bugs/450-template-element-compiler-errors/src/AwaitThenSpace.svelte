<script>let p = $state(Promise.resolve(1));</script>{#await p}x{ :then v}{v}{/await}
