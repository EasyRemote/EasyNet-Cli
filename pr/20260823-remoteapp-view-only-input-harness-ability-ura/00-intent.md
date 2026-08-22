# Intent — RemoteApp View-Only Input Harness Ability URA

The live frontend product-flow now reaches host view-only input safety after
Hub API, daemon runtime readiness, frontend typecheck/UI flow, permission
subject, target picker freshness, and decoded-frame window evidence pass.

The next failure is in the host E2E harness, not the RemoteApp plugin: the
harness passes the short ability name `remote_desktop.attach` to a CLI argument
that expects a full Ability URA. The product-flow should exercise the same
public governed ability surface as the frontend/backend path, so the harness
must resolve and use the advertised Ability URA/descriptor instead of a bare
method name.
