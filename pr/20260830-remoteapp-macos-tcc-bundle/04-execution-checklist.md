# Execution Checklist

- [x] Reproduce the permission denial with the exact debug helper path.
- [x] Verify System Settings refuses the selected flat Unix executable.
- [x] Verify the flat helper had an unstable ad-hoc cdhash identity.
- [x] Verify Developer ID signing alone still does not create a TCC entry.
- [x] Package a canonical macOS app with `Info.plist` and AppKit lifecycle.
- [x] Launch it through LaunchServices and transfer bounded lanes with
  `SCM_RIGHTS`.
- [x] Remove daemon/native-observer TCC checks made under the wrong identities.
- [x] Prove release signing and package-contract gates retain the matching ID.
- [x] Request and grant Screen Recording to the rebuilt physical helper.
- [x] Run permission, create/render/end, and terminal-receipt verification.
