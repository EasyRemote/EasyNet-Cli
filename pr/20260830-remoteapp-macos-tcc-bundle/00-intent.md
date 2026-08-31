# Intent

## Goal

Make the macOS RemoteApp ScreenCaptureKit owner a user-authorizable application
identity instead of an identity-less background executable. Preserve the flat
Runtime package on Linux and Windows; package a signed `.app` on macOS because
physical TCC evidence disproved both the flat executable and direct inner-binary
execution paths.

## Non-goals

- Do not move capture ownership into the frontend or EasyNet backend.
- Do not bypass TCC or synthesize successful capture when permission is absent.
- Do not change the Linux or Windows native-host layout.

## Acceptance criteria

- The media-host app carries
  `CFBundleIdentifier=run.easynet.remoteapp.media-host` and uses the same
  signing ID.
- The daemon launches the app through LaunchServices and transfers only its
  bounded private descriptors over a mode-0700 local-user socket.
- Release signing preserves that stable identity after final binary mutation.
- The media-host probe reports
  `granted=true`, and the real browser create/render/end lifecycle passes.
