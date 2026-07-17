export function load({ params }) {
	void params.nope;
	return { slug: params.slug };
}
