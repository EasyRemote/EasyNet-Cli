# Decisions Log

## 2026-07-23

- Moved FFI public tuple semantic validation before daemon transport.
- Added a shared authority metadata projection in admission core so FFI does not duplicate the authority grammar.
- Moved session-subject admission and authority-audience matching into authority metadata core and deleted the duplicate admission-facade implementation.
- Rejected `x-easynet-delegation` as generic test metadata; authority metadata keys now carry authority payloads only.
- Extended architecture and canonical v2 gates so future changes cannot remove the FFI tuple/authority gate silently.
