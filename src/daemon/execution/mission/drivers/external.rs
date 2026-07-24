// EasyNet CLI — External Agent Driver
// ===================================
//
// Generic subprocess driver for user-defined (`external`) agents. Unlike
// claude-code / codex, this driver is not bound to a specific binary: it
// runs the registered `command` + `args`, feeds the NL prompt on stdin,
// and returns stdout as the answer.

use crate::daemon::execution::mission::adapter::{
    AdapterOutput, AgentAdapter, DriverCommand, InvokeOpts,
};
use crate::daemon::execution::mission::process_runner::{self, ChildOptions};
use crate::daemon::persistence::agent_registry::AgentEntry;

/// Zero-sized singleton driver for `AgentType::External`.
pub(crate) struct ExternalAdapter;

impl AgentAdapter for ExternalAdapter {
    fn runtime_id(&self) -> &'static str {
        "external"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn invoke(
        &self,
        entry: &AgentEntry,
        prompt: &str,
        opts: InvokeOpts,
    ) -> anyhow::Result<AdapterOutput> {
        let entry_command = DriverCommand::from_registry_value(&entry.command);
        let command = entry_command.explicit().or_else(|| opts.command.explicit());
        let Some(command) = command else {
            anyhow::bail!(
                "external agent has no command configured; register it with \
                 `easynet agent add <name> --type external --command <program> [--arg ...]`"
            );
        };
        let args: Vec<&str> = entry.args.iter().map(String::as_str).collect();
        let result = process_runner::run_child(
            command,
            &args,
            ChildOptions {
                timeout: opts.timeout,
                max_stdout_bytes: opts.max_output_bytes,
                stdin_data: Some(prompt.to_string()),
                env: opts.env.clone(),
                cwd: Some(opts.cwd.clone()),
                ..Default::default()
            },
        )?;

        if result.exit_code != 0 {
            anyhow::bail!(
                "external agent command `{}` exited {}: {}",
                command,
                result.exit_code,
                result.stderr.trim()
            );
        }

        Ok(AdapterOutput {
            content: result.stdout,
            usage: None,
            tool_calls: Vec::new(),
            thread_id: None,
        })
    }
}
