// Current overlay shape — props collapsed to Record<string,any>
declare type Labels = { format?: (v: number) => string; placement?: 'inside' | 'outside' };

declare const BarChart_current: <TData>(
  __anchor: any,
  props: Partial<Record<string, any> & __SvnSvelte4PropsWiden<Record<string, any>> & __SvnAllProps>,
) => {
  data: TData[];
  labels: Labels | boolean;
};

{
  const __svn_CN = __svn_ensure_component(BarChart_current);
  new __svn_CN({
    target: __svn_any(),
    props: {
      data: [1, 2, 3],
      labels: { format: (value) => String(Math.abs(value)) }, // (value) should get contextual type
    },
  });
}
