<!-- Reviewer item #4a: literal slot attrs (`<slot foo="bar">`)
     should flow into the slot-def so consumers' `let:foo` destructure
     resolves `foo` to the literal string type. Pre-fix the
     `collect_slot_def` walker silently skipped every plain attr that
     wasn't `name=`, so `<slot kind="header">` lost the `kind: string`
     entry and consumers couldn't read it. -->
<slot kind="header" />
