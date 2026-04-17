<script lang="ts">
    import Switch from './Switch.svelte';

    let isOn = $state(false);
</script>

<!-- The destructure param `{ checked }` in the arrow handler must get its -->
<!-- type from Switch's declared `onchange: (event: { checked: boolean }) => void`. -->
<!-- If our overlay's default export is typed `any`, contextual typing degrades -->
<!-- and `checked` reads as 'implicit any' under strict mode. -->
<Switch
    checked={isOn}
    onchange={({ checked }) => (isOn = checked)} />
