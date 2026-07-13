// Broken companion to clean.ts — proves the synthesized
// `error: App.Error` annotation actually constrains the destructured
// value instead of falling through to `any`.
//
// Expected diagnostics (exactly two):
//   - TS2339 on `error.bogus` — 'bogus' does not exist on App.Error
//   - TS2322 on the `const wrong` assignment — App.Error's `message`
//     is a string, not assignable to a number-typed const.

declare function $props<P>(): P;

declare global {
    namespace App {
        interface Error {
            message: string;
        }
    }
}

async function $$render_fixture() {
    type $$ComponentProps = { error: App.Error; };

    let { error }: $$ComponentProps = $props();

    console.log(error.bogus); // TS2339
    const wrong: number = error.message; // TS2322
    void wrong;
    return { props: undefined as any as $$ComponentProps };
}
$$render_fixture;
export {};
