export const actions = {
  default: async ({ request }) => {
    const d = await request.formData();
    return { ok: true, name: d.get('x') };
  },
  wrong: () => 5,
};
export function entries() {
  return [{ id: 'a' }, { nope: 1 }];
}
export const load = async ({ params }) => ({ id: params.id });
