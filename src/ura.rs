// EasyNet-Cli
// ===========
//
// File: src/ura.rs
// Description: CLI façade for Axon-owned URA builders and parser.
//
// URA is protocol state owned by Axon. This file deliberately contains
// no grammar implementation and no string construction logic; it only
// re-exports `easynet_axon::ura` so existing CLI modules can keep using
// `crate::ura::*` while the source of truth remains in Axon SDK.
//
// Canonical shapes, all built by Axon:
//
//   user      easynet:///r/<realm>/user/<user-id>
//   device    easynet:///r/<realm>/device/<device-id>
//   agent     easynet:///r/<realm>/agent/<user-id>.<agent-id>
//   ability   easynet:///r/<realm>/ability/<owner>.<namespace>.<ability-id>
//   hub       easynet:///r/<realm>/hub
//   resource  easynet:///r/<realm>/resource/<owner-id>/<path>
//
// Examples:
//
//   easynet:///r/localhost/device/8315ea5c-7cfd-473e-8fef-95340af6d971
//   easynet:///r/localhost/agent/u-9f4.frontend-engineer
//   easynet:///r/localhost/ability/u-9f4.frontend-engineer.chat
//   easynet:///r/localhost/ability/hub.federation.resolve
//   easynet:///r/localhost/resource/agent.u-9f4.frontend-engineer/skill/alive-video
//
// CLI-specific rule:
//
//   When a CLI feature needs a URA, call one of the re-exported Axon
//   builders below. Do not add `format!("easynet:///r/...")`,
//   `strip_prefix("easynet:///r/")`, or a parallel parser in CLI code.
//   The guard at `tests/scripts/test_no_raw_ura_construction.sh` exists
//   to keep that invariant enforceable.

pub use easynet_axon::ura::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_uses_axon_sdk_ura_builder() {
        assert_eq!(
            ability_ura("localhost", "hub", "federation", "resolve"),
            "easynet:///r/localhost/ability/hub.federation.resolve"
        );
        assert_eq!(
            resource_dot_ura(
                "localhost",
                "agent.dev.frontend-engineer",
                "skill/alive-video"
            ),
            "easynet:///r/localhost/resource/agent.dev.frontend-engineer/skill/alive-video"
        );
    }
}
