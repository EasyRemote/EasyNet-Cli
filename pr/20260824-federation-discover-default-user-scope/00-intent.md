# Intent

Make `easynet federation discover` safe and useful on a paired device.

The concrete failure is a healthy, admitted device whose product health audit
fails because the command treats an omitted `--user-id` as an unfiltered
operator/audit read and then attempts that Authority-only read from a Device
runtime. The device already has an authenticated user binding, so its default
directory view must use that user scope.

Acceptance criteria:

- A paired device defaults to its credential-bound User directory projection.
- An unfiltered read requires an explicit `--operator-audit` request.
- Operator/audit invocation rejects a non-Authority local daemon before I/O.
- `--user-id` remains available for an explicit user-scoped diagnostic.
- The local runtime audit can observe the admitted device through the same
  privacy boundary used by product surfaces.
