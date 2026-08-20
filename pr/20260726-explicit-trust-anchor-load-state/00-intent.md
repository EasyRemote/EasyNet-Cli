# Intent

Remove the legacy `RealmTrustAnchor::load_or_empty` compatibility layer.

The trust-anchor storage loader must report storage state exactly. Policy
decisions such as "first boot may continue without a trust-anchor file" belong
at the daemon boot boundary, not inside the canonical trust-anchor model.
