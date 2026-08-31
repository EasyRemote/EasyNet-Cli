# Invariants

- A physical camera has one active producer per Runtime resource.
- Snapshot never returns a stale frame from an inactive producer.
- Cold snapshot acquisition has a finite timeout.
- Snapshot receipt and typed JPEG payload remain compatible.
