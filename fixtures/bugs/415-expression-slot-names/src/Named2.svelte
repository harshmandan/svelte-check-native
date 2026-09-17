<script lang="ts">let n = "a";</script><slot name={n} x={1}></slot><slot {name} y="2"></slot><slot name="a{n}" z></slot>
<script context="module" lang="ts">let name = "q";</script>
