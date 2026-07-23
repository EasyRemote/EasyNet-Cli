# Risk

This change intentionally fails start/attach earlier when key-service custody is broken. That can surface more startup errors in dirty local environments, but it prevents a worse product state where the daemon reports ready and fails later through descriptor resolution or invocation history.

The proof helper uses the existing canonical signer abstraction, so it should not create a second identity authority or product-specific signer model.

