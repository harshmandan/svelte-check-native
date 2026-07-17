<script lang="ts">
    let note = 'hello';
    function shout(s: string): string { return s.toUpperCase(); }
</script>

<textarea>Line one <br> line two {shout(note)} and {shout(42)}</textarea>
