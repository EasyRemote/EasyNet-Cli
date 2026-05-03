# Intent

Fix the three CLI coverage failures confirmed on 2026-05-03:

1. `easynet device show <cross-hub-node>` must resolve a remote device by
   `node_id`, not fail on a local-only view.
2. `easynet auth abilities <cross-hub-node>` must not hard-fail on backend
   HTTP 404 when the local daemon can resolve the device through federation.
3. `easynet ability exec <same-hub-node> -- ...` must route through the
   working federation invoke path instead of the removed `fleet.exec_remote`
   transport.

The repair stays inside EasyNet-Cli. We do not require backend changes for
operator-visible success on this repo's acceptance surface.
