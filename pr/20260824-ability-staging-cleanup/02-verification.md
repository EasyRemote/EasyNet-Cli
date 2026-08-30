# Verification

```bash
cargo fmt --all -- --check
cargo test -p easynet --features axon-pb deploy_ability_ --lib
cargo build -p easynet --features axon-pb --bin easynet-daemon --bin easynet
```

The paired daemon was rebuilt and restarted. A real EasyRemote deploy + raw
stream + uninstall smoke passed while the count of
`tmp/easynet-ability-deploy/*.tar.gz` remained unchanged before and after the
run.
