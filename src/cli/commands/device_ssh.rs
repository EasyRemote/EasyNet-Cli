// EasyNet CLI - native terminal and opt-in OpenSSH adapter
// =======================================================
//
// File: src/cli/commands/device_ssh.rs
// Description: Keep EasyNet terminal as the default remote-access surface;
//              expose OpenSSH only as an explicit adapter over net.tunnel.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;

use anyhow::Context;

pub(crate) fn run(
    device: &str,
    openssh: bool,
    alias: Option<&str>,
    remote_host: &str,
    remote_port: u16,
    user: Option<&str>,
    config: Option<&PathBuf>,
    arguments: &[String],
) -> anyhow::Result<()> {
    if !openssh {
        if alias.is_some()
            || remote_host != "127.0.0.1"
            || remote_port != 22
            || user.is_some()
            || config.is_some()
            || !arguments.is_empty()
        {
            anyhow::bail!("SSH_ADAPTER_OPT_IN_REQUIRED: pass --openssh for OpenSSH options");
        }
        return super::device_terminal::run(device);
    }
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(device)?;
    let destination_alias = alias
        .map(validate_alias)
        .transpose()?
        .unwrap_or("easynet-target");
    validate_remote_host(remote_host)?;
    validate_remote_port(remote_port)?;
    let user = user.map(validate_user).transpose()?;
    let executable = std::env::current_exe().context("resolve easynet executable")?;
    let proxy_command = format!(
        "{} device proxy {} %h %p",
        shell_quote(&executable.display().to_string()),
        shell_quote(&target_ura),
    );
    let mut command = std::process::Command::new("ssh");
    if let Some(config) = config {
        command.arg("-F").arg(config);
    }
    command
        .arg("-o")
        .arg(format!("ProxyCommand={proxy_command}"))
        .arg("-o")
        .arg(format!("HostName={remote_host}"))
        .arg("-p")
        .arg(remote_port.to_string());
    let destination = user
        .map(|user| format!("{user}@{destination_alias}"))
        .unwrap_or_else(|| destination_alias.to_string());
    command.arg(destination).args(arguments);
    let status = command.status().context("run system OpenSSH client")?;
    if !status.success() {
        anyhow::bail!("OpenSSH exited with {status}");
    }
    Ok(())
}

pub(crate) fn print_config(
    device: &str,
    alias: &str,
    remote_host: &str,
    remote_port: u16,
    user: Option<&str>,
) -> anyhow::Result<()> {
    let alias = validate_alias(alias)?;
    validate_remote_host(remote_host)?;
    validate_remote_port(remote_port)?;
    let user = user.map(validate_user).transpose()?;
    let target_ura = crate::support::platform::remote_device::resolve_target_device_ura(device)?;
    let executable = std::env::current_exe().context("resolve easynet executable")?;
    println!("Host {alias}");
    println!("    HostName {remote_host}");
    println!("    Port {remote_port}");
    if let Some(user) = user {
        println!("    User {user}");
    }
    println!(
        "    ProxyCommand {} device proxy {} %h %p",
        shell_quote(&executable.display().to_string()),
        shell_quote(&target_ura),
    );
    println!("    ProxyUseFdpass no");
    println!();
    println!("# This alias can be used as `ProxyJump {alias}` by another Host stanza.");
    Ok(())
}

fn validate_alias(value: &str) -> anyhow::Result<&str> {
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-@".contains(character))
    {
        anyhow::bail!("INVALID_SSH_ALIAS: alias may contain only ASCII alphanumerics, . _ - @");
    }
    Ok(value)
}

fn validate_user(value: &str) -> anyhow::Result<&str> {
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        anyhow::bail!("INVALID_SSH_USER: user may contain only ASCII alphanumerics, . _ -");
    }
    Ok(value)
}

fn validate_remote_host(value: &str) -> anyhow::Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-:".contains(character))
    {
        anyhow::bail!("INVALID_SSH_HOST: remote host contains unsupported characters");
    }
    Ok(())
}

fn validate_remote_port(value: u16) -> anyhow::Result<()> {
    if value == 0 {
        anyhow::bail!("INVALID_SSH_PORT: remote port must be between 1 and 65535");
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_handles_spaces_and_quotes() {
        assert_eq!(shell_quote("a b'c"), "'a b'\\''c'");
    }

    #[test]
    fn ssh_alias_rejects_shell_syntax() {
        assert!(validate_alias("host;touch").is_err());
    }

    #[test]
    fn ssh_user_and_port_fail_closed_before_process_launch() {
        assert!(validate_user("owner@other").is_err());
        assert!(validate_remote_port(0).is_err());
    }
}
