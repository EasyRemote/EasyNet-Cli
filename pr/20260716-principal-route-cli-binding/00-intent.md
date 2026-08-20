Principal route CLI binding convergence

Goal

Promote PrincipalLifecycle provider route facts out of SDK-private ownership
and generate Rust CLI bindings from the same manifest used by Go and Python.
The operator-facing CLI syntax, payloads, daemon dispatch behavior, and public
SDK APIs remain unchanged.

Expected effect

- Architecture convergence: PrincipalLifecycle ability names have one
  provider-route source across SDK facades and the Rust CLI facade.
- Boundary clarity: the manifest is a repository provider-route artifact, not
  a language SDK artifact.
- Maintenance: adding or renaming a PrincipalLifecycle route requires one
  manifest edit plus regeneration.
