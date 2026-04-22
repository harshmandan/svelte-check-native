declare type __SvnSvelte4PropsWiden<P> = 'children' extends keyof P
    ? {}
    : { children?: any };
declare type __SvnAllProps = { [index: string]: any };
declare type __SvnPropsPartial<P> = { [K in keyof P]?: P[K] | null };

declare function __svn_any(x?: any): any;

declare function __svn_ensure_component<P extends Record<string, any>>(
    c: (anchor: any, props: P) => any,
): new (options: { target?: any; props?: __SvnPropsPartial<P> }) => { $$prop_def: P };
declare function __svn_ensure_component<P>(
    c: (anchor: any, props: P) => any,
): new (options: { target?: any; props?: P }) => { $$prop_def: P };
declare function __svn_ensure_component(
    c: unknown,
): new (options: { target?: any; props?: any }) => { $$prop_def: any };
