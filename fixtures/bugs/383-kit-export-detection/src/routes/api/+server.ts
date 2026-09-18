// A default-exported handler is an export under its own name.
export default function GET(e) { return new Response(e.nope); }
