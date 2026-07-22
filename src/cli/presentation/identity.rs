// EasyNet CLI — identity projection presentation
// ===============================================
//
// File: src/cli/presentation/identity.rs
// Description: CLI-only rendering projection for credential-bound runtime
//              identities.
//
// Protocol Responsibility
// -----------------------
// Keep product surfaces honest about runtime user identity state. A paired
// device may be bound to a canonical User URA, explicitly device-only, or
// invalid. Presentation code must not collapse these states into an omitted
// row.
//
// Implementation Approach
// -----------------------
// The daemon persistence layer owns credential interpretation through
// `RuntimeUserBinding`. This module owns only the CLI display projection that
// status, auth, and banner share.
//
// Usage Contract
// --------------
// Call `runtime_user_binding_display` whenever a CLI surface needs the
// "Current user" row for paired credentials. Product presentation code must
// not silently discard user-URA projection errors.
//
// Architectural Position
// ----------------------
// CLI presentation layer. It depends on persistence domain types but contains
// no daemon admission, signer, route, or descriptor logic.

use crate::daemon::persistence::config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeUserBindingDisplayState {
    Bound,
    Unbound,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeUserBindingDisplay {
    value: String,
    state: RuntimeUserBindingDisplayState,
}

impl RuntimeUserBindingDisplay {
    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn state(&self) -> RuntimeUserBindingDisplayState {
        self.state
    }
}

pub fn runtime_user_binding_display(creds: &config::Credentials) -> RuntimeUserBindingDisplay {
    match creds.runtime_user_binding() {
        Ok(config::RuntimeUserBinding::Bound { user_ura }) => RuntimeUserBindingDisplay {
            value: user_ura,
            state: RuntimeUserBindingDisplayState::Bound,
        },
        Ok(config::RuntimeUserBinding::Unbound { reason }) => RuntimeUserBindingDisplay {
            value: reason.to_string(),
            state: RuntimeUserBindingDisplayState::Unbound,
        },
        Err(error) => RuntimeUserBindingDisplay {
            value: format!("invalid ({error:#})"),
            state: RuntimeUserBindingDisplayState::Invalid,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credentials(
        user_id: Option<&str>,
        join_receipt_hash: Option<String>,
    ) -> config::Credentials {
        config::Credentials {
            node_id: "device-a".to_string(),
            credential_token: if join_receipt_hash.is_some() {
                String::new()
            } else {
                "token".to_string()
            },
            hub_endpoint: "https://hub.example:50443".to_string(),
            realm: "localhost".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: user_id.map(|_| "alice".to_string()),
            user_id: user_id.map(str::to_string),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash,
        }
    }

    #[test]
    fn display_projects_bound_user_ura() {
        let display = runtime_user_binding_display(&credentials(Some("alice"), None));

        assert_eq!(display.state(), RuntimeUserBindingDisplayState::Bound);
        assert_eq!(display.value(), "easynet:///r/localhost/user/alice");
    }

    #[test]
    fn display_projects_unbound_federation_native_state() {
        let display = runtime_user_binding_display(&credentials(None, Some("a".repeat(64))));

        assert_eq!(display.state(), RuntimeUserBindingDisplayState::Unbound);
        assert_eq!(
            display.value(),
            "not bound (federation-native device credential)"
        );
    }

    #[test]
    fn display_projects_invalid_user_binding_state() {
        let display = runtime_user_binding_display(&credentials(Some("   "), Some("a".repeat(64))));

        assert_eq!(display.state(), RuntimeUserBindingDisplayState::Invalid);
        assert!(
            display.value().contains("missing user_id"),
            "invalid display should retain projection error: {display:?}"
        );
    }
}
