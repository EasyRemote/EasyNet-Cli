# Decisions Log

## 2026-08-30 — Launch a signed media-host application identity

Apple documents both that designated requirements identify access to
privacy-protected resources. Physical tests showed that System Settings would
not add the flat Unix executable and that directly executing an app's inner
binary still ran outside the authorized application context. The canonical
macOS boundary is therefore a signed `.app` launched through LaunchServices.
The private process protocol remains unchanged; `SCM_RIGHTS` bootstraps the
existing bounded descriptors without moving capture ownership.

Rejected:

- Repeated `CGRequestScreenCaptureAccess` from an identity-less flat helper.
- TCC database mutation or other permission bypass.
- Granting capture to the browser/frontend instead of the media owner.
- Directly executing the app's inner binary after signing.
- Checking Screen Recording permission from the daemon or target observer,
  whose TCC identities differ from the media owner.
