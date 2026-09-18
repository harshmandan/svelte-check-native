<script>import C from "./C.svelte"; function f() {}</script>
<C>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div onclick={f}></div>
  <div onclick={f}></div>
</C>
<div>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div onclick={f}></div>
  <div onclick={f}></div>
</div>
