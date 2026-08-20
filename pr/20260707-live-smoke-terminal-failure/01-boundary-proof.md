# Boundary Proof

## Runtime Boundary

The implementation exercises typed terminal failure as a canonical runtime invocation result:

- Go decodes `InvocationResult.OK() == false`, `TerminalState() == "Failed"`, and `Failure() != nil`.
- Python decodes `InvocationResult.ok is False`, `terminal_state == "Failed"`, and `error is not None`.

The smoke uses the existing public `RuntimeClient.invoke` / `RuntimeClient.Invoke` APIs. No product-specific SDK capability is introduced.

## Failure Trigger

The live failure trigger is a prepared `observe.health` invocation submitted with an intentionally invalid external Ed25519 signature. `SubmitSigned` returns the SDK observation handle before daemon execution; `Await` then returns the terminal runtime failure envelope. The SDK assertion remains generic and validates only the runtime failure envelope.

## Architecture Constraints

- URA terminology is preserved.
- No legacy input alias is introduced.
- Go and Python converge on the same capability matrix item: typed terminal failure is provider-backed in both SDK live smokes.
- Existing file-transfer bidi coverage remains under `fs.transfer`; self-tests now gate that coverage explicitly.
