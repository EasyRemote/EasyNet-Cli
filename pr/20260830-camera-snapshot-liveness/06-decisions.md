# Decisions

- Do not extend the 15 second `AVCapturePhotoOutput` wait: the callback path is
  non-delivering in the daemon, so a longer timeout only prolongs false loading.
- Do not keep the old photo-output path as fallback; one producer is the
  canonical camera ownership model.
