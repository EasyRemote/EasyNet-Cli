//! Seven-axes W1 — `easynet discover` end-to-end
//! ===============================================
//!
//! File: tests/seven_axes_w1_discover_e2e.rs
//! Spec: docs/spec/seven-axes-p0-landing-v1.md §3 W1-E2E-1/2.
//! Fixture: `seven_axes_fixture` (real UDS daemon + product-file
//! seeded HOME — see that module's header for the stack).
//!
//! Covered here (W1-E2E-2 in full; W1-E2E-1 single-daemon subset):
//!   * ladder-entry resolution over the wire
//!     (`agent.list` → `discover` with `testbot` as selected callee);
//!   * typed degradation: an unjoined daemon answers
//!     `federation_not_joined` — never an error — local tiers intact;
//!   * candidate projection: URA round-trips the Axon parser,
//!     owner_kind comes typed, and the score reproduces the frozen
//!     ranking contract digit-for-digit;
//!   * the seven-tuple audit surface: the report carries the
//!     invocation envelope echo (spec 0.1-7).
//!
//! Still tracked elsewhere: cross-owner projection through the user
//! tier needs the two-daemon hub fixture
//! (`cross_hub_two_daemon_real_tls_e2e.rs` pattern).
//!
//! One `#[test]` on purpose — fixture owns process env (see fixture
//! header).
//!
//! Author: Silan Hu <silan.hu@u.nus.edu>
//! Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use std::path::Path;
use std::time::Duration;

use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest};
use easynet_cli::facade::cli::discover::{
    self, DiscoverArgs, DiscoverScopeMode, OutputFormat, SourceWindowMode,
};
use easynet_cli::persistence::config;
use seven_axes_fixture::SevenAxesHome;
use tonic::transport::{Channel, Endpoint, Uri};

const REMOTE_PUBLIC_NAME: &str = "remote-file-reader";
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

fn args(intent: &str) -> DiscoverArgs {
    DiscoverArgs {
        intent: intent.to_string(),
        limit: 15,
        scope: DiscoverScopeMode::Realm,
        as_agent: None,
        tree: false,
        source_window: SourceWindowMode::Bounded,
        format: OutputFormat::Table,
    }
}

async fn connect_to_daemon(socket_path: &Path) -> Channel {
    let socket_path = socket_path.to_path_buf();
    Endpoint::try_from("http://[::]:50051")
        .expect("dummy endpoint")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let path = socket_path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await
        .expect("connect to daemon")
}

fn invoke_daemon_ability(
    socket_path: &Path,
    caller_ura: &str,
    function_name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    rt.block_on(async {
        let mut client = InvocationClient::new(connect_to_daemon(socket_path).await);
        let response = tokio::time::timeout(
            STEP_TIMEOUT,
            client.invoke(tonic::Request::new(InvokeRequest {
                envelope: Some(Envelope {
                    caller: Some(AgentIdentity {
                        ura: caller_ura.to_string(),
                        ..AgentIdentity::default()
                    }),
                    callee: Some(AgentIdentity {
                        ura: caller_ura.to_string(),
                        ..AgentIdentity::default()
                    }),
                    invocation_nonce: vec![0x51; 16],
                    ..Envelope::default()
                }),
                function_name: function_name.to_string(),
                arguments: serde_json::to_vec(&args).expect("encode daemon invoke args"),
                ..InvokeRequest::default()
            })),
        )
        .await
        .expect("daemon invoke must not hang")
        .expect("daemon invoke must succeed")
        .into_inner();
        serde_json::from_slice(&response.result).expect("daemon result must be JSON")
    })
}

fn advertise_remote_user_tier_ability(socket_path: &Path, host_device_ura: &str) -> String {
    let owner_ura = easynet_cli::ura::agent_ura("cli", "local", "remote-worker");
    let ability_ura = easynet_cli::ura::owner_ability_ura(&owner_ura, REMOTE_PUBLIC_NAME)
        .expect("mint remote ability URA");

    invoke_daemon_ability(
        socket_path,
        host_device_ura,
        "federation.advertise_agent",
        serde_json::json!({
            "agent_ura": owner_ura,
            "public_key_hex": "",
            "signing_authority": {
                "kind": "hosted_by",
                "host_ura": host_device_ura,
            },
            "host_node_id": "remote-node",
        }),
    );
    invoke_daemon_ability(
        socket_path,
        host_device_ura,
        "federation.advertise_abilities",
        serde_json::json!({
            "owner_ura": owner_ura,
            "host_device_ura": host_device_ura,
            "projection_revision": 1,
            "projection_digest": "remote-worker-v1",
            "lease_expires_unix_ms": 0,
            "ability_summaries": [{
                "ability_ura": ability_ura,
                "owner_ura": owner_ura,
                "namespace": "remote",
                "local_name": REMOTE_PUBLIC_NAME,
                "descriptor_revision": "v1",
                "schema_ref": null,
                "schema_hash": null,
                "policy_ref": "visibility:PUBLIC",
                "route_summary_ref": null,
                "tags": ["remote", "file"],
                "callable_summary": {
                    "public_name": REMOTE_PUBLIC_NAME,
                    "description": "read a remote file from another owner",
                    "ability_class": "tool",
                    "input_fields": [],
                    "flags": {
                        "read_only": true,
                        "destructive": false,
                        "idempotent": true,
                        "streaming_only": false,
                        "bidi_only": false,
                    }
                }
            }],
        }),
    );

    ability_ura
}

#[test]
fn discover_e2e_local_scope_and_typed_federation_degradation() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon();

    // ── W1-E2E-2: unjoined federation degrades typed ─────────────────
    let joined_credentials =
        config::load_credentials().expect("fixture should seed joined credentials");
    config::delete_credentials().expect("temporarily unjoin the fixture");
    let report = discover::execute(&args("chat")).expect("discover executes against live daemon");
    assert_eq!(
        report.tiers_searched,
        vec!["device"],
        "user tier must not be listed when federation is degraded"
    );
    let fed = report
        .federation
        .as_ref()
        .expect("degradation must surface as a typed status object");
    assert_eq!(
        fed.status, "federation_not_joined",
        "unjoined daemon must degrade as federation_not_joined; got {fed:?}"
    );
    assert!(
        report.invocations.len() >= 2,
        "report must carry both local and realm-tier invocation echoes; got {:?}",
        report.invocations
    );
    assert!(
        report.invocations.iter().all(|meta| meta.is_object()),
        "every envelope echo must be a structured object; got {:?}",
        report.invocations
    );
    config::save_credentials(&joined_credentials).expect("restore joined credentials");

    // ── W1-E2E-1 (single-daemon subset): candidate projection ────────
    let mut self_args = args("weather forecast");
    self_args.as_agent = Some("testbot".into());
    let weather = discover::execute(&self_args).expect("discover executes for the seeded ability");
    let candidate = weather
        .candidates
        .iter()
        .find(|c| c.name.ends_with("weather-probe"))
        .unwrap_or_else(|| panic!("seeded ability must rank: {:?}", weather.candidates));

    let selector = easynet_cli::ura::AbilitySelector::parse(&candidate.ura)
        .expect("candidate URA must round-trip the Axon parser");
    assert_eq!(candidate.owner_kind, "agent");
    assert_eq!(selector.owner_kind(), "agent");
    assert_eq!(
        candidate.scope, "self",
        "the ladder is testbot's own; its ability sits in the self tier"
    );

    // Frozen ranking contract, recomputed by hand for this fixture
    // (spec W1-E2E-1 ③ — a user can predict every row's score):
    //   "weather": name hit 3 + segment-prefix 2 + description 1
    //              + owner(URA) 1                          = 7
    //   "forecast": description 1                          = 1
    //   every token hit somewhere, 2 tokens → bonus        = 2
    assert_eq!(
        candidate.score, 10,
        "score must follow the frozen name×3(+2)/desc×1/owner×1/+2 contract"
    );
    assert_eq!(weather.skipped_unparseable, 0, "nothing may drop silently");

    // ── W1-E2E-1 user tier: same daemon acting as local hub ──────────
    //
    // This is the cross-owner discover path without the expensive
    // two-binary TLS harness: the test writes the hub read models
    // through the public federation advertise abilities, then the
    // normal `<agent>.discover(scope=user)` path calls
    // `federation.resolve` over the daemon Invocation surface.
    let remote_ura = advertise_remote_user_tier_ability(&home.socket_path, &home.loopback_caller);
    let user_scope =
        discover::execute(&args("remote file")).expect("discover user tier through local hub");
    assert_eq!(
        user_scope.tiers_searched,
        vec!["device", "user"],
        "joined daemon must list the user tier"
    );
    assert!(
        user_scope.federation.is_none(),
        "joined daemon must not surface federation degradation: {:?}",
        user_scope.federation
    );
    let remote = user_scope
        .candidates
        .iter()
        .find(|c| c.ura == remote_ura)
        .unwrap_or_else(|| {
            panic!(
                "remote user-tier ability must rank: {:?}",
                user_scope.candidates
            )
        });
    assert_eq!(remote.scope, "user");
    assert_eq!(remote.owner_kind, "agent");

    drop(daemon);
}
