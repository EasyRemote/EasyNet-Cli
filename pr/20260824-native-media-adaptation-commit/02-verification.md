# Verification

Passed:

```text
CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p easynet --lib native::tests --features axon-pb
CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p easynet --lib webrtc_native_media::tests --features axon-pb
CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 cargo test -p easynet --lib videotoolbox_encoder::tests --features axon-pb
bash tests/scripts/test_remoteapp_media_adaptation_e2e.sh
bash tools/scripts/check-remoteapp-product-closure-audit.sh
bash tests/scripts/test_check_remoteapp_product_closure_audit.sh
cargo fmt --all -- --check
git diff --check
```

The first build attempt failed before tests because the data volume had only
959 MiB free (`No space left on device`). `cargo clean -p easynet` removed
7.8 GiB of rebuildable package output; the same focused tests then passed.

Expected fail-closed product-status check:

```text
bash tools/scripts/remoteapp-product-completion-e2e.sh --check --out-dir <temp>
```

Exit status was `1`, `product_complete_claim` was `false`, and all 18 required
live report inputs were absent. This is the correct current product status; the
focused implementation tests above do not substitute for those reports.
