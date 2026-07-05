//! ABI-root feature discovery catalog.
//!
//! This module owns the language-neutral Runtime Core feature matrix returned by
//! `easynet_feature_discovery`. Language facades decode this DTO; they do not
//! maintain an independent capability list.

use serde_json::{json, Value};

use super::EASYNET_ABI_VERSION;

pub const PROFILES: &[(&str, &str)] = &[
    ("runtime_core", "partial"),
    ("receipt", "fetch_projection_partial"),
    ("directory_identity", "read_model_projection_partial"),
    ("publication", "carrier_partial"),
    ("host_binding", "codec_partial"),
    ("mission", "carrier_status_partial"),
    ("events", "directory_session_stream_partial"),
    ("admin_gateway", "carrier_status_partial"),
    ("surface", "carrier_projection_partial"),
    ("compatibility", "carrier_projection_partial"),
    ("wrappers", "carrier_record_projection_partial"),
];

pub const ALWAYS_ON_SYMBOLS: &[&str] = &[
    "daemon_lifecycle",
    "invocation_dispatch_v3",
    "typed_error_json",
    "receipt_fetch",
    "receipt_projection",
    "directory_identity_projection",
    "identity_signing_key_lifecycle",
    "directory_read_model",
    "directory_resolve",
    "host_binding_codec",
    "publication_carriers",
    "mission_carriers",
    "mission_status_projection",
    "events_directory_stream",
    "events_session_stream",
    "admin_gateway_carriers",
    "admin_gateway_status_projection",
    "admin_device_session_projection",
    "admin_device_admin_projection",
    "surface_carriers",
    "surface_projection",
    "surface_health",
    "compatibility_carriers",
    "compatibility_projection",
    "compatibility_file_adapters",
    "wrapper_carriers",
    "wrapper_record_projection",
];

pub const AXON_PB_SYMBOLS: &[&str] = &[
    "invocation_builder_handles",
    "invocation_handle_observation",
    "stream_bidi_lifecycle",
    "runtime_health",
    "prepare_sign_submit",
];

pub fn feature_discovery_value() -> Value {
    let profiles = PROFILES
        .iter()
        .map(|(name, status)| ((*name).to_owned(), json!(status)))
        .collect::<serde_json::Map<String, Value>>();
    let mut symbols = ALWAYS_ON_SYMBOLS
        .iter()
        .map(|name| ((*name).to_owned(), json!(true)))
        .collect::<serde_json::Map<String, Value>>();
    for name in AXON_PB_SYMBOLS {
        symbols.insert((*name).to_owned(), json!(cfg!(feature = "axon-pb")));
    }

    json!({
        "abi_version": EASYNET_ABI_VERSION,
        "sdk_version": env!("CARGO_PKG_VERSION"),
        "profiles": profiles,
        "symbols": symbols,
        "axon_pb": cfg!(feature = "axon-pb")
    })
}

pub fn feature_discovery_json() -> String {
    feature_discovery_value().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_catalog_reports_all_declared_profiles_and_symbols() {
        let value = feature_discovery_value();
        assert_eq!(value["abi_version"], EASYNET_ABI_VERSION);
        assert_eq!(value["sdk_version"], env!("CARGO_PKG_VERSION"));

        for (profile, status) in PROFILES {
            assert_eq!(value["profiles"][*profile], *status);
        }
        for symbol in ALWAYS_ON_SYMBOLS {
            assert_eq!(value["symbols"][*symbol], true);
        }
        for symbol in AXON_PB_SYMBOLS {
            assert_eq!(value["symbols"][*symbol], json!(cfg!(feature = "axon-pb")));
        }
        assert_eq!(value["axon_pb"], json!(cfg!(feature = "axon-pb")));
    }

    #[test]
    fn feature_catalog_matches_shared_conformance_fixture() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("sdk/conformance/fixtures/feature-discovery.v4.json");
        let fixture: Value =
            serde_json::from_slice(&std::fs::read(fixture_path).expect("read feature fixture"))
                .expect("decode feature fixture");

        assert_eq!(fixture["abi_version"], EASYNET_ABI_VERSION);
        assert_eq!(fixture["sdk_version"], env!("CARGO_PKG_VERSION"));
        for (profile, status) in PROFILES {
            assert_eq!(fixture["profiles"][*profile], *status);
        }
        for symbol in ALWAYS_ON_SYMBOLS {
            assert_eq!(fixture["symbols"][*symbol], true);
        }
        for symbol in AXON_PB_SYMBOLS {
            assert!(
                fixture["symbols"].get(*symbol).is_some(),
                "fixture missing axon-pb gated symbol {symbol}"
            );
        }
    }
}
