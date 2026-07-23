# Decisions Log

- Decision: fix the tests by provisioning explicit credentials, not by relaxing
  `local_device_ura()` or `resource_ref_for_local_path`.
  Rationale: production local device ownership must remain mandatory; the defect
  is the fixture boundary, not the identity resolver.
