// Minimal stand-in for SvelteKit's `$app/forms` so the fixture can
// compile without the full Kit package. The shapes mirror
// SubmitFunction + enhance closely enough to exercise the contextual-
// typing emit.

export type SubmitFunction<
    Success = Record<string, unknown> | undefined,
    Failure = Record<string, unknown> | undefined,
> = (input: {
    action: URL;
    formData: FormData;
    formElement: HTMLFormElement;
    controller: AbortController;
    submitter: HTMLElement | null;
    cancel: () => void;
}) =>
    | void
    | ((opts: {
          result: { type: 'success'; data: Success } | { type: 'failure'; data: Failure };
          update: (options?: { reset?: boolean; invalidateAll?: boolean }) => Promise<void>;
      }) => void | Promise<void>);

export declare function enhance<
    Success = Record<string, unknown> | undefined,
    Failure = Record<string, unknown> | undefined,
>(
    form_element: HTMLFormElement,
    submit?: SubmitFunction<Success, Failure>,
): { destroy(): void };
