// Gap C repro — assignment-narrowing inside template-check closures.
//
// User pattern (Threlte/theatre Project.svelte):
//
//   interface Props { project?: IProject; children?: Snippet<[{ project: IProject }]> }
//   let { project = $bindable(), children }: Props = $props()
//   project = pool.get() ?? createProject()    // narrows project to IProject
//   { @render children?.({ project }) }         // template — uses narrowed project
//
// In our overlay, the template-check is wrapped in:
//
//   async function __svn_tpl_check() { ... project ... }
//
// while upstream uses:
//
//   async () => { ... project ... };
//
// The difference matters for TS's control-flow narrowing of the
// outer `project` reassignment. We want to find the shape that
// preserves narrowing.

interface IProject {
    name: string;
    ready: Promise<void>;
}

declare function pool_get(name: string): IProject | undefined;
declare function create_project(name: string): IProject;

function expectIProject(p: IProject): void {
    void p;
}

// Variant A — our current shape: `async function NAME() {}` declaration.
function $$render_ours(props: { project?: IProject }): void {
    let { project }: { project?: IProject } = props;
    project = pool_get('a') ?? create_project('a');

    async function __svn_tpl_check() {
        // EXPECT FAIL — closure narrowing doesn't carry into the
        // declared async function below the reassignment.
        expectIProject(project);
    }
    void __svn_tpl_check;
}

// Variant B — upstream shape: `async () => {}` arrow expression
// statement (no name, expression context, never assigned to a
// variable).
function $$render_upstream(props: { project?: IProject }): void {
    let { project }: { project?: IProject } = props;
    project = pool_get('b') ?? create_project('b');

    async () => {
        // EXPECT CLEAN — closure narrowing carries through the
        // arrow expression.
        expectIProject(project);
    };
}

// Variant C — same as B but as an IIFE (invoked).
function $$render_iife(props: { project?: IProject }): void {
    let { project }: { project?: IProject } = props;
    project = pool_get('c') ?? create_project('c');

    void (async () => {
        expectIProject(project);
    });
}

void $$render_ours;
void $$render_upstream;
void $$render_iife;
