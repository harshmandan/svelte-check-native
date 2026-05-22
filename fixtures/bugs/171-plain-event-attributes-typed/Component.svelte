<script lang="ts">
    // Svelte 5 plain DOM event attributes (no on: directive). These
    // flow through svelteHTML.createElement and pick up the element's
    // typed event-handler shape (MouseEventHandler, SubmitEventHandler,
    // …) so a wrong-arity / wrong-arg handler must fire TS2322.
    //
    // Locks the contract for the runes-era event idiom. If a future
    // emit change stops forwarding plain on*-attributes into the
    // createElement call, the "BAD" lines below will stop erroring
    // and this fixture will fail. Companion to fixture 170.

    const click_ok = (_: MouseEvent) => {}
    const submit_ok = (_: SubmitEvent) => {}
    const input_ok = (_: Event) => {}
    const change_ok = (_: Event) => {}
    const keydown_ok = (_: KeyboardEvent) => {}

    const wrong_arg: (e: number) => void = (_) => {}
</script>

<button onclick={click_ok}>OK</button>
<form onsubmit={submit_ok}>
    <input oninput={input_ok} onchange={change_ok} onkeydown={keydown_ok} />
</form>

<!-- BAD: wrong arg type. -->
<button onclick={wrong_arg}>BAD</button>
