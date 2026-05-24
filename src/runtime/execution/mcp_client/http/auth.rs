// EasyNet CLI — Streamable HTTP / auth header injection
// =====================================================
//
// File: src/runtime/execution/mcp_client/http/auth.rs
//
// Applies the per-server [`AuthSpec`] to an outgoing request
// builder. One implementation drives both the POST request path
// and the GET listener reconnect path so a single config entry
// covers every outbound direction.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use hyper::header::AUTHORIZATION;

use crate::runtime::execution::mcp_client::AuthSpec;

/// Apply the per-server auth credentials to an outgoing request
/// builder. Returns the builder with the appropriate headers
/// installed; `AuthSpec::BearerEnv` looks up the env var at call
/// time and surfaces a typed error if it is not set.
pub(super) fn apply_auth_headers(
    mut req_builder: hyper::http::request::Builder,
    auth: Option<&AuthSpec>,
) -> anyhow::Result<hyper::http::request::Builder> {
    let Some(auth) = auth else {
        return Ok(req_builder);
    };
    match auth {
        AuthSpec::Bearer { token } => {
            req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        AuthSpec::BearerEnv { env } => {
            let token = std::env::var(env).with_context(|| {
                format!(
                    "AuthSpec::BearerEnv references env var `{env}` which is not set; \
                     the daemon must inherit it or the operator must export it"
                )
            })?;
            req_builder = req_builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        AuthSpec::Headers { headers } => {
            for (name, value) in headers {
                req_builder = req_builder.header(name.as_str(), value.as_str());
            }
        }
    }
    Ok(req_builder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::Request;
    use std::collections::HashMap;

    fn build() -> hyper::http::request::Builder {
        Request::builder().method("POST").uri("http://localhost/mcp")
    }

    #[test]
    fn no_auth_leaves_request_unchanged() {
        let req = apply_auth_headers(build(), None)
            .unwrap()
            .body(Vec::<u8>::new())
            .unwrap();
        assert!(req.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    fn auth_bearer_appends_authorization_header() {
        let req = apply_auth_headers(
            build(),
            Some(&AuthSpec::Bearer {
                token: "abc123".into(),
            }),
        )
        .unwrap()
        .body(Vec::<u8>::new())
        .unwrap();
        assert_eq!(
            req.headers().get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer abc123"
        );
    }

    #[test]
    fn auth_bearer_env_reads_from_environment() {
        std::env::set_var("OP_LOG_TEST_BEARER_TOKEN", "from-env-456");
        let req = apply_auth_headers(
            build(),
            Some(&AuthSpec::BearerEnv {
                env: "OP_LOG_TEST_BEARER_TOKEN".into(),
            }),
        )
        .unwrap()
        .body(Vec::<u8>::new())
        .unwrap();
        assert_eq!(
            req.headers().get(AUTHORIZATION).unwrap().to_str().unwrap(),
            "Bearer from-env-456"
        );
        std::env::remove_var("OP_LOG_TEST_BEARER_TOKEN");
    }

    #[test]
    fn auth_bearer_env_missing_var_fails_with_clear_message() {
        std::env::remove_var("OP_LOG_TEST_DEFINITELY_NOT_SET");
        let err = apply_auth_headers(
            build(),
            Some(&AuthSpec::BearerEnv {
                env: "OP_LOG_TEST_DEFINITELY_NOT_SET".into(),
            }),
        )
        .expect_err("missing env var must surface a typed error");
        let msg = format!("{err:#}");
        assert!(msg.contains("OP_LOG_TEST_DEFINITELY_NOT_SET"));
        assert!(msg.contains("not set"));
    }

    #[test]
    fn auth_headers_map_injects_arbitrary_pairs() {
        let mut headers = HashMap::new();
        headers.insert("X-Api-Key".to_string(), "key-789".to_string());
        headers.insert("X-Tenant".to_string(), "acme".to_string());
        let req = apply_auth_headers(build(), Some(&AuthSpec::Headers { headers }))
            .unwrap()
            .body(Vec::<u8>::new())
            .unwrap();
        assert_eq!(req.headers().get("X-Api-Key").unwrap(), "key-789");
        assert_eq!(req.headers().get("X-Tenant").unwrap(), "acme");
    }
}
