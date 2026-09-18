<script>import C from "./C.svelte"; function f() {}</script>
<C>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div slot="a" onclick={f}></div>
  <div onclick={f}></div>
</C>
