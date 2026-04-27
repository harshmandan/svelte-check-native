// Reproduces the Threlte InstancedMeshes pattern — uses
// `Parameters<typeof Instance>` and `(typeof Instance)[]`.

import { InstanceOurs } from './instance_ours.ts';
import { InstanceUpstream } from './instance_upstream.ts';
import { InstanceComponent } from './instance_component.ts';

// ---- Shape 1: OURS — currently fires errors. ----

const getInstanceOurs = (id: string) => {
    return (...args: Parameters<typeof InstanceOurs>) => {
        return InstanceOurs(...args);
    };
};

// EXPECT FAIL with current OURS shape.
function getArrayOurs(): (typeof InstanceOurs)[] {
    return [getInstanceOurs('a'), getInstanceOurs('b')];
}

// ---- Shape 2: UPSTREAM per-component iso. ----

const getInstanceUp = (id: string) => {
    return (...args: Parameters<typeof InstanceUpstream>) => {
        return InstanceUpstream(...args);
    };
};

// EXPECT FAIL too — upstream's per-component iso also has a `new` sig
// the inner arrow can't satisfy.
function getArrayUp(): (typeof InstanceUpstream)[] {
    return [getInstanceUp('a'), getInstanceUp('b')];
}

// ---- Shape 3: UPSTREAM Component<P, X, B> — the actual `Instance.svelte` shape. ----

const getInstanceComp = (id: string) => {
    return (...args: Parameters<typeof InstanceComponent>) => {
        return InstanceComponent(...args);
    };
};

// EXPECT CLEAN — Component<> has only a call signature, no `new`.
function getArrayComp(): (typeof InstanceComponent)[] {
    return [getInstanceComp('a'), getInstanceComp('b')];
}

void getArrayOurs;
void getArrayUp;
void getArrayComp;
