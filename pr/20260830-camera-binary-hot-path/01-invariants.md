# Invariants

1. Preview storage is O(1) frames per active resource/configuration.
2. Preview work discarded by backpressure is dropped before base64 or JSON
   allocation.
3. Every admitted preview frame carries exact `content_type=image/jpeg` bytes.
4. Payload bytes never bypass Runtime sequence, receipt, cancellation, or
   terminal lifecycle.
5. Recording persistence has O(1) media-buffer memory and uses an atomic
   same-volume commit when possible.
6. Context index publication occurs only after the destination media artifact
   exists; an index failure removes the uncommitted artifact.
7. Temporary recording files are removed on every terminal failure.
8. Snapshot result bytes and `image/jpeg` content type are bound together in
   the canonical terminal receipt and every transport projection.
9. Snapshot metadata is not duplicated into the JPEG payload and no base64 or
   JSON representation of image bytes is allocated on the daemon hot path.
