// EasyNet CLI
// ===========
//
// File: src/cli/federation_gen_cert.rs
// Description: `easynet federation gen-cert` — generate a TLS cert
//              chain shaped for cross-hub federation: a self-signed
//              CA root + a leaf signed by that CA, with SAN/CN
//              wired correctly for rustls.
//
// Why this command exists
// -----------------------
// The cross-hub dialer pins the peer's CA via `tls_ca_pem_path`
// and rustls (the underlying TLS implementation) refuses to use a
// CA cert directly as the end-entity cert it presents on the wire
// (`OtherError(CaUsedAsEndEntity)`). An operator who ran
// `openssl req -x509` once to make a single self-signed cert and
// pointed both `tls_cert_pem` and the peer's `tls_ca_pem_path` at
// it would hit that error — surfaced as the #1 pitfall in the real
// production pair-flow run.
//
// This command produces the four files the operator needs:
//   <out-dir>/ca.pem      — self-signed CA root (peer trusts this)
//   <out-dir>/ca.key      — CA private key (kept locally; rotate to
//                           re-issue leaves)
//   <out-dir>/cert.pem    — leaf+CA concatenated; what the daemon
//                           presents as `tls_cert_pem`
//   <out-dir>/key.pem     — leaf private key; what the daemon uses
//                           as `tls_key_pem`
//
// Implementation
// --------------
// We shell out to `openssl` rather than pulling in `rcgen` as a
// runtime dependency. The demo scripts already use the same
// commands; the cross-hub demo's GREEN run is the reference for
// the exact invocation shape.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};

use clap::Args;

use crate::support::output;

#[derive(Debug, Args)]
pub struct GenCertArgs {
    /// Output directory for the generated cert files. Created if
    /// missing.
    #[arg(long)]
    pub out_dir: PathBuf,
    /// Common Name (CN) and SAN entry for the leaf cert. Use the
    /// hostname or IP the peer dialer will see — `localhost` for
    /// loopback-only demos, the public DNS name for a real
    /// deployment.
    #[arg(long, default_value = "localhost")]
    pub cn: String,
    /// Validity in days. Defaults to 30 — short enough to force a
    /// rotation cadence in production, long enough to survive a
    /// single demo run.
    #[arg(long, default_value_t = 30)]
    pub days: u32,
}

pub fn run(args: GenCertArgs) -> anyhow::Result<()> {
    require_openssl()?;
    std::fs::create_dir_all(&args.out_dir)?;

    let ca_pem = args.out_dir.join("ca.pem");
    let ca_key = args.out_dir.join("ca.key");
    let leaf_csr = args.out_dir.join("leaf.csr");
    let leaf_pem = args.out_dir.join("leaf.pem");
    let leaf_ext = args.out_dir.join("leaf.ext");
    let cert_pem = args.out_dir.join("cert.pem");
    let key_pem = args.out_dir.join("key.pem");

    output::info(&format!("Generating CA root in {}", ca_pem.display()));
    run_openssl(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        &ca_key.display().to_string(),
        "-out",
        &ca_pem.display().to_string(),
        "-days",
        &args.days.to_string(),
        "-subj",
        &format!("/CN={}-ca", args.cn),
    ])?;

    output::info("Generating leaf CSR + key");
    run_openssl(&[
        "req",
        "-new",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-keyout",
        &key_pem.display().to_string(),
        "-out",
        &leaf_csr.display().to_string(),
        "-subj",
        &format!("/CN={}", args.cn),
    ])?;

    write_leaf_extfile(&leaf_ext, &args.cn)?;

    output::info("Signing leaf with CA");
    run_openssl(&[
        "x509",
        "-req",
        "-in",
        &leaf_csr.display().to_string(),
        "-CA",
        &ca_pem.display().to_string(),
        "-CAkey",
        &ca_key.display().to_string(),
        "-CAcreateserial",
        "-out",
        &leaf_pem.display().to_string(),
        "-days",
        &args.days.to_string(),
        "-extfile",
        &leaf_ext.display().to_string(),
    ])?;

    concat_leaf_and_ca(&leaf_pem, &ca_pem, &cert_pem)?;

    let _ = std::fs::remove_file(&leaf_csr);
    let _ = std::fs::remove_file(&leaf_pem);
    let _ = std::fs::remove_file(&leaf_ext);
    let _ = std::fs::remove_file(args.out_dir.join("ca.srl"));

    set_owner_only_mode(&ca_key)?;
    set_owner_only_mode(&key_pem)?;

    output::success(&format!(
        "Generated cert chain in {}",
        args.out_dir.display()
    ));
    output::detail("ca.pem", &ca_pem.display().to_string());
    output::detail("ca.key", &ca_key.display().to_string());
    output::detail("cert.pem", &cert_pem.display().to_string());
    output::detail("key.pem", &key_pem.display().to_string());
    eprintln!();
    print_daemon_config_snippet(&cert_pem, &key_pem);
    eprintln!();
    print_realm_trust_snippet(&ca_pem);
    Ok(())
}

fn require_openssl() -> anyhow::Result<()> {
    let probe = std::process::Command::new("openssl")
        .arg("version")
        .output();
    match probe {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => anyhow::bail!(
            "openssl exited non-zero: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => anyhow::bail!("openssl not found on PATH: {e}\n  install openssl and re-run."),
    }
}

fn run_openssl(args: &[&str]) -> anyhow::Result<()> {
    let out = std::process::Command::new("openssl").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "openssl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

fn write_leaf_extfile(path: &Path, cn: &str) -> anyhow::Result<()> {
    let body =
        format!("subjectAltName=DNS:localhost,DNS:{cn},IP:127.0.0.1\nbasicConstraints=CA:FALSE\n");
    std::fs::write(path, body)?;
    Ok(())
}

fn concat_leaf_and_ca(leaf: &Path, ca: &Path, dest: &Path) -> anyhow::Result<()> {
    let mut buf = std::fs::read(leaf)?;
    let ca_bytes = std::fs::read(ca)?;
    if !buf.ends_with(b"\n") {
        buf.push(b'\n');
    }
    buf.extend_from_slice(&ca_bytes);
    std::fs::write(dest, buf)?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_mode(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_mode(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn print_daemon_config_snippet(cert_pem: &Path, key_pem: &Path) {
    eprintln!("Add to ~/.easynet/daemon-config.toml under [daemon]:");
    eprintln!();
    eprintln!("  tls_cert_pem = \"{}\"", cert_pem.display());
    eprintln!("  tls_key_pem  = \"{}\"", key_pem.display());
}

fn print_realm_trust_snippet(ca_pem: &Path) {
    eprintln!("On the peer hub, point its [[trusted_agent]] block at:");
    eprintln!();
    eprintln!("  tls_ca_pem_path = \"{}\"", ca_pem.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skip when openssl is not available so this test can run in
    /// minimal CI environments (e.g. Alpine containers without
    /// openssl-bin). Production CI image has openssl, so this only
    /// affects local-dev edge cases.
    fn openssl_available() -> bool {
        std::process::Command::new("openssl")
            .arg("version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn gen_cert_emits_four_files_with_correct_chain_shape() {
        if !openssl_available() {
            eprintln!("openssl not on PATH — skipping gen_cert test");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let args = GenCertArgs {
            out_dir: dir.path().to_path_buf(),
            cn: "test.example".into(),
            days: 1,
        };
        run(args).expect("gen-cert succeeds");

        for name in &["ca.pem", "ca.key", "cert.pem", "key.pem"] {
            let p = dir.path().join(name);
            assert!(p.exists(), "{} should exist", p.display());
            let bytes = std::fs::read(&p).expect("read");
            assert!(!bytes.is_empty(), "{} should not be empty", p.display());
        }

        // cert.pem must contain TWO certificates (leaf + CA).
        let cert_bytes = std::fs::read_to_string(dir.path().join("cert.pem")).expect("read cert");
        let begin_count = cert_bytes.matches("-----BEGIN CERTIFICATE-----").count();
        assert_eq!(
            begin_count, 2,
            "cert.pem must be a 2-cert chain (leaf + CA), got {begin_count}",
        );

        // Intermediate / scratch files must be cleaned up.
        for name in &["leaf.csr", "leaf.pem", "leaf.ext", "ca.srl"] {
            let p = dir.path().join(name);
            assert!(!p.exists(), "{} should have been removed", p.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn gen_cert_keys_have_owner_only_permissions() {
        if !openssl_available() {
            eprintln!("openssl not on PATH — skipping gen_cert perms test");
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let args = GenCertArgs {
            out_dir: dir.path().to_path_buf(),
            cn: "test.example".into(),
            days: 1,
        };
        run(args).expect("gen-cert succeeds");

        for name in &["ca.key", "key.pem"] {
            let p = dir.path().join(name);
            let mode = std::fs::metadata(&p).expect("stat").permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "{} should be 0600, got {:o}",
                p.display(),
                mode & 0o777
            );
        }
    }
}
