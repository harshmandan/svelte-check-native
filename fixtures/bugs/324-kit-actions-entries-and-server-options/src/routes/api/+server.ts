export const prerender = 'sometimes';
export const trailingSlash = 'maybe';
export const entries = () => [{ x: 1 }];
export async function GET({ url }) {
  return new Response(url.pathname);
}
