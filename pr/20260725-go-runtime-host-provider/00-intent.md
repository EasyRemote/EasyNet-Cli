# Intent

## Goal

Move the remaining Go runtime host lifecycle provider seam out of
`sdk/go/provider/easynet` into the product-neutral `sdk/go/provider/runtime`
package, and remove the Go EasyNet daemon credentials identity adapter.

## Non-goals

- Do not keep `provider/easynet` import aliases.
- Do not preserve product credential projection helpers inside the SDK.
- Do not change runtime lifecycle state-machine behavior.

## Acceptance criteria

- Go runtime host lifecycle provider code lives under `provider/runtime`.
- Go tests import the runtime provider path.
- `sdk/go/provider/easynet` is deleted.
- SDK conformance inventory classifies the Go provider under the runtime provider owner.
- Product-neutrality and SPEC v2 gates reject the retired Go EasyNet provider path.
