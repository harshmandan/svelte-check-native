// Mimics how our emit WOULD produce a TS-overlay containing both
// genuine user-code diagnostics AND scaffolding-region diagnostics.
// The filter's job at the mapper layer is to drop the scaffolding
// diagnostics and surface the user-code ones.
//
// tsgo fires TS2322 on BOTH marked lines when run standalone. Our
// mapper, after implementing the ignore-region filter, must:
//   - surface the "user" TS2322 at line `user_err` (no markers)
//   - drop the "scaffold" TS2322 at line `scaffold_err` (inside markers)

async function $$render() {
    // --- user-code region (ERROR must surface) ---
    const user: string = (null as any as string | null)!;
    // Actually the above doesn't fire because of `!`. Use a direct
    // mismatch the way consumer call sites would.
    declare_user_err();

    // --- generated scaffolding region (ERROR must be dropped) ---
    /*svn:ignore_start*/
    const scaffold: string = null as any as string | undefined;
    // ^ TS2322: Type 'string | undefined' is not assignable to type 'string'.
    void scaffold;
    /*svn:ignore_end*/

    // --- user-code region (ERROR must surface) ---
    const user_err: string = null as any as string | undefined;
    // ^ TS2322: Type 'string | undefined' is not assignable to type 'string'.
    void user_err;
}

function declare_user_err(): void {}

void $$render;
