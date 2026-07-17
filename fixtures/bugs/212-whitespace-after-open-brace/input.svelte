<script lang="ts">
    const items: string[] = ['a'];
    let flag = true;
    let markup = '<b>x</b>';
</script>

{ #if flag}
    <p>on</p>
{ :else}
    <p>off</p>
{ /if}

{ #each items as item}
    <li>{item}</li>
{ /each}

{ @html markup}

{ const doubled = items.length * 2}
{doubled}
