<script lang="ts">
    // Threlte/theatre Project.svelte pattern: prop is optional, then
    // reassigned in the script body to a non-undefined value, then
    // referenced in the template. Pre-Gap-C our template-check
    // wrapper was a NAMED `async function` declaration which TS
    // hoists; hoisting collapsed flow-narrowing back to the union
    // type, so `project` was `IProject | undefined` inside the
    // closure. Switching to `;(async () => {...});` (arrow expression
    // statement) preserves narrowing — `project` is `IProject` after
    // the `?? createProject(...)` assignment.

    interface IProject {
        ready: Promise<void>;
        name: string;
    }

    interface Props {
        name?: string;
        project?: IProject;
        children?: import('svelte').Snippet<[{ project: IProject }]>;
    }

    function pool_get(_n: string): IProject | undefined {
        return undefined;
    }
    function create_project(name: string): IProject {
        return { ready: Promise.resolve(), name };
    }

    let { name = 'default', project = $bindable(), children }: Props = $props();

    project = pool_get(name) ?? create_project(name);
</script>

{#await project.ready then}
    {@render children?.({ project })}
{/await}
