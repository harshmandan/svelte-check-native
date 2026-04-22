// Proposed overlay shape — props slot uses the same typed record as the
// callable's return type. Contextual typing flows into `format: (value) =>`.
declare type Labels = { format?: (v: number) => string; placement?: 'inside' | 'outside' };

type BarChartProps<TData> = {
  data: TData[];
  labels: Labels | boolean;
};

declare const BarChart_fixed: <TData>(
  __anchor: any,
  props: Partial<BarChartProps<TData> & __SvnSvelte4PropsWiden<BarChartProps<TData>> & __SvnAllProps>,
) => BarChartProps<TData>;

{
  const __svn_CN = __svn_ensure_component(BarChart_fixed);
  new __svn_CN({
    target: __svn_any(),
    props: {
      data: [1, 2, 3],
      labels: { format: (value) => String(Math.abs(value)) }, // value: number
    },
  });
}

// Broken-check companion: wrong return type in labels.format.
{
  const __svn_CN = __svn_ensure_component(BarChart_fixed);
  new __svn_CN({
    target: __svn_any(),
    props: {
      data: [1, 2, 3],
      // @ts-expect-error value is number, not string — Math.abs(value) would
      // be a type error if the contextual type flows.
      labels: { format: (value) => value.toUpperCase() },
    },
  });
}
