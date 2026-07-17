<svelte:head>
    <style>
        div {
            color: blue;
        }
    </style>
</svelte:head>

<svg>
    <style>.dot { fill: red; }</style>
</svg>

<!-- comment survives -->
{@html '<b>raw</b>'}
{@const answer = 42}
{@debug answer}
