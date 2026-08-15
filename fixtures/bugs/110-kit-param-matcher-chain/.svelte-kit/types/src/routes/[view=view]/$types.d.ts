type MatcherParam<M> = M extends (param : string) => param is (infer U extends string) ? U : string;
export type RouteParams = { view: MatcherParam<typeof import('../../../../../src/params/view.js').match> };
