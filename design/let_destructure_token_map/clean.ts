// Validates that destructuring an unknown field fires TS2339, AND that
// when our overlay adds a TokenMap entry for the bound name, the
// diagnostic survives our mapper rather than being dropped.
//
// This fixture only exercises tsgo behavior — confirms the destructure
// pattern fires the diagnostic in the first place. Mapper survival is
// a Rust-side test (not a tsgo concern).

declare const slot_def: { a: boolean; b: string };
const { a, b: c, d } = slot_def;  // expect TS2339 on `d`
void a;
void c;
void d;
