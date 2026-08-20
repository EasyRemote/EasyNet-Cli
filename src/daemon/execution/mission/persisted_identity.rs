// EasyNet CLI — Mission Persisted Identity Facts
// ==============================================
//
// File: src/daemon/execution/mission/persisted_identity.rs
// Description: Shared serde boundary for persisted mission runtime identity
//              facts.
//
// Protocol Responsibility:
//   Runtime persisted state must carry explicit, non-empty identity facts.
//   Readers must not silently synthesize trace or invocation identities from
//   directory names or legacy defaults.
//
// Implementation Approach:
//   Provide one focused serde deserializer used by mission meta records that
//   need a required identity string.
//
// Usage Contract:
//   Use this at persisted-state boundaries only. In-memory builders should
//   carry typed context directly and should not depend on serde validation.
//
// Architectural Position:
//   Daemon runtime execution layer; not a public SDK or product facade.

use serde::Deserialize;

pub(crate) fn deserialize_non_empty_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.trim().is_empty() {
        return Err(serde::de::Error::custom(
            "runtime identity fact must be a non-empty string",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    struct RequiredIdentity {
        #[serde(deserialize_with = "deserialize_non_empty_string")]
        value: String,
    }

    #[test]
    fn required_identity_accepts_non_empty_string() {
        let parsed: RequiredIdentity =
            serde_json::from_str(r#"{ "value": "invocation-1" }"#).expect("parse identity");
        assert_eq!(parsed.value, "invocation-1");
    }

    #[test]
    fn required_identity_rejects_empty_string() {
        let error = serde_json::from_str::<RequiredIdentity>(r#"{ "value": "" }"#)
            .expect_err("empty identity must fail closed");
        assert!(
            error
                .to_string()
                .contains("runtime identity fact must be a non-empty string"),
            "unexpected error: {error}"
        );
    }
}
