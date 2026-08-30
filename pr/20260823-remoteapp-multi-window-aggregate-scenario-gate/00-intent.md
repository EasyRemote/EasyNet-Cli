# RemoteApp multi-window aggregate scenario gate

## Intent

Tighten the RemoteApp product-completion gate so multi-window and
multi-application tracking cannot be inferred from coverage flags alone.

The dedicated `remoteapp-multi-window-tracking-e2e.sh` verifier remains the
owner of live tracking artifacts. This slice makes the product-completion gate
require structured scenario summaries before it accepts that report as product
completion evidence.

## Product gap closed

Readiness evidence may include explicit unsupported states. Product completion
must not. In particular, `multi_display_application` must pass with
`MultiAppSurface=true`; an explicit unsupported state is useful readiness
evidence but cannot satisfy the final RemoteApp product-complete claim.
