//! High-level human/agent input translated to CDP commands.
//! =======================================================
//!
//! File: plugins/browser/src/input.rs
//! Description: Strict human-scale browser event parsing and CDP translation.
//!
//! Protocol Responsibility:
//! - Keep shared-page input target-scoped and free of caller-selected routing.
//!
//! Implementation Approach:
//! - Validate exact fields, bring the governed page forward, and emit the
//!   smallest CDP command sequence for each event kind.
//!
//! Usage Contract:
//! - Callers provide one already-authorized session and one bounded event.
//!
//! Architectural Position:
//! - Browser plugin application service between handlers and the session.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use url::Url;

use super::constants::{MAX_INPUT_TEXT_BYTES, MAX_KEY_BYTES, MAX_SELECTOR_BYTES, MAX_URL_BYTES};
use super::errors::{BrowserError, BrowserResult};
use super::session::BrowserSession;

pub async fn apply_input(
    session: Arc<BrowserSession>,
    ability: &'static str,
    event: Value,
) -> BrowserResult<Value> {
    let object = event
        .as_object()
        .ok_or_else(|| invalid(ability, "`event` must be an object"))?;
    let kind = required_string(object, "kind", 32, ability)?;
    validate_event_fields(&kind, object, ability)?;
    session
        .ensure_foreground()
        .await
        .map_err(|error| remap_ability(error, ability))?;

    let result = match kind.as_str() {
        "navigate" => navigate(&session, object, ability).await?,
        "click" => click(&session, object, ability).await?,
        "mouse" => mouse(&session, object, ability).await?,
        "scroll" => scroll(&session, object, ability).await?,
        "keydown" => key(&session, object, ability, "keyDown").await?,
        "keyup" => key(&session, object, ability, "keyUp").await?,
        "text" => text(&session, object, ability).await?,
        "fill" => fill(&session, object, ability).await?,
        other => {
            return Err(invalid(
                ability,
                format!("unsupported event kind {other:?}"),
            ))
        }
    };
    Ok(json!({
        "kind": kind,
        "applied": true,
        "result": result,
    }))
}

fn validate_event_fields(
    kind: &str,
    event: &Map<String, Value>,
    ability: &'static str,
) -> BrowserResult<()> {
    let allowed = match kind {
        "navigate" => &["kind", "url"][..],
        "click" => &["kind", "selector", "x", "y", "button"][..],
        "mouse" => &["kind", "action", "x", "y", "button"][..],
        "scroll" => &["kind", "x", "y", "delta_x", "delta_y"][..],
        "keydown" | "keyup" => &["kind", "key"][..],
        "text" => &["kind", "text"][..],
        "fill" => &["kind", "selector", "value"][..],
        other => {
            return Err(invalid(
                ability,
                format!("unsupported event kind {other:?}"),
            ))
        }
    };
    let unknown = event
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(invalid(
            ability,
            format!(
                "unsupported field(s) for {kind} event: {}",
                unknown.join(", ")
            ),
        ));
    }
    if kind == "click" && event.contains_key("selector") {
        let coordinate_fields = ["x", "y", "button"]
            .into_iter()
            .filter(|field| event.contains_key(*field))
            .collect::<Vec<_>>();
        if !coordinate_fields.is_empty() {
            return Err(invalid(
                ability,
                format!(
                    "selector click cannot include coordinate field(s): {}",
                    coordinate_fields.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

async fn navigate(
    session: &BrowserSession,
    event: &Map<String, Value>,
    ability: &'static str,
) -> BrowserResult<Value> {
    let url = required_string(event, "url", MAX_URL_BYTES, ability)?;
    let parsed =
        Url::parse(&url).map_err(|error| invalid(ability, format!("invalid url: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(invalid(ability, "navigate url must be absolute http(s)"));
    }
    session
        .command("Page.navigate", Some(json!({"url": url})))
        .await
        .map_err(|error| remap_ability(error, ability))
}

async fn click(
    session: &BrowserSession,
    event: &Map<String, Value>,
    ability: &'static str,
) -> BrowserResult<Value> {
    if let Some(selector) = optional_string(event, "selector", MAX_SELECTOR_BYTES, ability)? {
        return evaluate_dom_action(
            session,
            ability,
            "click",
            format!(
                "(() => {{ const el = document.querySelector({}); if (!el) return {{ok:false,reason:'not_found'}}; el.scrollIntoView({{block:'center',inline:'center'}}); el.click(); return {{ok:true}}; }})()",
                serde_json::to_string(&selector).expect("selector JSON")
            ),
        )
        .await;
    }
    let x = required_number(event, "x", ability)?;
    let y = required_number(event, "y", ability)?;
    let button = optional_button(event, ability, "left")?;
    session
        .command(
            "Input.dispatchMouseEvent",
            Some(json!({"type":"mousePressed","x":x,"y":y,"button":button,"clickCount":1})),
        )
        .await
        .map_err(|error| remap_ability(error, ability))?;
    session
        .command(
            "Input.dispatchMouseEvent",
            Some(json!({"type":"mouseReleased","x":x,"y":y,"button":button,"clickCount":1})),
        )
        .await
        .map_err(|error| remap_ability(error, ability))
}

async fn mouse(
    session: &BrowserSession,
    event: &Map<String, Value>,
    ability: &'static str,
) -> BrowserResult<Value> {
    let action = required_string(event, "action", 32, ability)?;
    let event_type = match action.as_str() {
        "move" => "mouseMoved",
        "down" => "mousePressed",
        "up" => "mouseReleased",
        other => {
            return Err(invalid(
                ability,
                format!("unsupported mouse action {other:?}"),
            ))
        }
    };
    let x = required_number(event, "x", ability)?;
    let y = required_number(event, "y", ability)?;
    let default_button = if action == "move" { "none" } else { "left" };
    let button = optional_button(event, ability, default_button)?;
    session
        .command(
            "Input.dispatchMouseEvent",
            Some(json!({"type":event_type,"x":x,"y":y,"button":button,"clickCount":1})),
        )
        .await
        .map_err(|error| remap_ability(error, ability))
}

async fn scroll(
    session: &BrowserSession,
    event: &Map<String, Value>,
    ability: &'static str,
) -> BrowserResult<Value> {
    let x = optional_number(event, "x", 0.0, ability)?;
    let y = optional_number(event, "y", 0.0, ability)?;
    let delta_x = optional_number(event, "delta_x", 0.0, ability)?;
    let delta_y = optional_number(event, "delta_y", 0.0, ability)?;
    session
        .command(
            "Input.dispatchMouseEvent",
            Some(json!({
                "type":"mouseWheel",
                "x":x,
                "y":y,
                "deltaX":delta_x,
                "deltaY":delta_y,
            })),
        )
        .await
        .map_err(|error| remap_ability(error, ability))
}

async fn key(
    session: &BrowserSession,
    event: &Map<String, Value>,
    ability: &'static str,
    event_type: &str,
) -> BrowserResult<Value> {
    let key = required_literal_string(event, "key", MAX_KEY_BYTES, ability)?;
    session
        .command(
            "Input.dispatchKeyEvent",
            Some(json!({"type":event_type,"key":key,"text": if event_type == "keyDown" && key.chars().count() == 1 { key.clone() } else { String::new() }})),
        )
        .await
        .map_err(|error| remap_ability(error, ability))
}

async fn text(
    session: &BrowserSession,
    event: &Map<String, Value>,
    ability: &'static str,
) -> BrowserResult<Value> {
    let text = bounded_string(event, "text", MAX_INPUT_TEXT_BYTES, ability)?;
    session
        .command("Input.insertText", Some(json!({"text": text})))
        .await
        .map_err(|error| remap_ability(error, ability))
}

async fn fill(
    session: &BrowserSession,
    event: &Map<String, Value>,
    ability: &'static str,
) -> BrowserResult<Value> {
    let selector = required_string(event, "selector", MAX_SELECTOR_BYTES, ability)?;
    let value = bounded_string(event, "value", MAX_INPUT_TEXT_BYTES, ability)?;
    let selector_json = serde_json::to_string(&selector).expect("selector JSON");
    let value_json = serde_json::to_string(&value).expect("value JSON");
    let expression = format!(
        "(() => {{ const el = document.querySelector({selector_json}); if (!el) return {{ok:false,reason:'not_found'}}; el.focus(); const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value')?.set; if (setter) setter.call(el, {value_json}); else el.value = {value_json}; el.dispatchEvent(new Event('input', {{bubbles:true}})); el.dispatchEvent(new Event('change', {{bubbles:true}})); return {{ok:true}}; }})()",
    );
    evaluate_dom_action(session, ability, "fill", expression).await
}

async fn evaluate_dom_action(
    session: &BrowserSession,
    ability: &'static str,
    action: &str,
    expression: String,
) -> BrowserResult<Value> {
    let evaluated = session
        .command(
            "Runtime.evaluate",
            Some(json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
                "userGesture": true,
            })),
        )
        .await
        .map_err(|error| remap_ability(error, ability))?;
    let value = evaluated
        .get("result")
        .and_then(|result| result.get("value"))
        .cloned()
        .unwrap_or(Value::Null);
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        return Err(invalid(
            ability,
            format!("{action} target was not found in the active document"),
        ));
    }
    Ok(value)
}

fn optional_button(
    event: &Map<String, Value>,
    ability: &'static str,
    default: &str,
) -> BrowserResult<String> {
    let button = event
        .get("button")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| invalid(ability, "`event.button` must be a string"))
        })
        .transpose()?
        .unwrap_or(default);
    if matches!(button, "none" | "left" | "middle" | "right") {
        Ok(button.to_string())
    } else {
        Err(invalid(
            ability,
            format!("unsupported mouse button {button:?}"),
        ))
    }
}

fn required_string(
    event: &Map<String, Value>,
    key: &str,
    max_bytes: usize,
    ability: &'static str,
) -> BrowserResult<String> {
    let value = event
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(ability, format!("`event.{key}` is required")))?;
    if value.len() > max_bytes {
        return Err(invalid(
            ability,
            format!("`event.{key}` exceeds {max_bytes} bytes"),
        ));
    }
    Ok(value.to_string())
}

fn required_literal_string(
    event: &Map<String, Value>,
    key: &str,
    max_bytes: usize,
    ability: &'static str,
) -> BrowserResult<String> {
    let value = event
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(ability, format!("`event.{key}` is required")))?;
    if value.len() > max_bytes {
        return Err(invalid(
            ability,
            format!("`event.{key}` exceeds {max_bytes} bytes"),
        ));
    }
    Ok(value.to_string())
}

fn optional_string(
    event: &Map<String, Value>,
    key: &str,
    max_bytes: usize,
    ability: &'static str,
) -> BrowserResult<Option<String>> {
    let Some(value) = event.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(ability, format!("`event.{key}` must be non-empty")))?;
    if value.len() > max_bytes {
        return Err(invalid(
            ability,
            format!("`event.{key}` exceeds {max_bytes} bytes"),
        ));
    }
    Ok(Some(value.to_string()))
}

fn bounded_string<'a>(
    event: &'a Map<String, Value>,
    key: &str,
    max_bytes: usize,
    ability: &'static str,
) -> BrowserResult<&'a str> {
    let value = event
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(ability, format!("`event.{key}` must be a string")))?;
    if value.len() > max_bytes {
        Err(invalid(
            ability,
            format!("`event.{key}` exceeds {max_bytes} bytes"),
        ))
    } else {
        Ok(value)
    }
}

fn required_number(
    event: &Map<String, Value>,
    key: &str,
    ability: &'static str,
) -> BrowserResult<f64> {
    event.get(key).and_then(Value::as_f64).ok_or_else(|| {
        invalid(
            ability,
            format!("`event.{key}` is required and must be numeric"),
        )
    })
}

fn optional_number(
    event: &Map<String, Value>,
    key: &str,
    default: f64,
    ability: &'static str,
) -> BrowserResult<f64> {
    event
        .get(key)
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| invalid(ability, format!("`event.{key}` must be numeric")))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn invalid(ability: &'static str, detail: impl Into<String>) -> BrowserError {
    BrowserError::InvalidArgument {
        ability,
        detail: detail.into(),
    }
}

fn remap_ability(error: BrowserError, ability: &'static str) -> BrowserError {
    match error {
        BrowserError::Cdp { detail, .. } => BrowserError::Cdp { ability, detail },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_parser_is_bounded_to_cdp_values() {
        assert_eq!(
            optional_button(&json!({}).as_object().unwrap().clone(), "test", "left").unwrap(),
            "left"
        );
        assert!(optional_button(
            &json!({"button":"side"}).as_object().unwrap().clone(),
            "test",
            "left"
        )
        .is_err());
    }

    #[test]
    fn key_parser_preserves_the_space_key_literal() {
        let event = json!({"key": " "});
        let parsed = required_literal_string(
            event.as_object().expect("event object"),
            "key",
            MAX_KEY_BYTES,
            "browser.send_input",
        )
        .expect("space is a valid key literal");
        assert_eq!(parsed, " ");
    }

    #[test]
    fn event_fields_are_strict_for_each_input_kind() {
        let navigate = json!({"kind":"navigate","url":"https://example.com","x":1});
        let navigate = navigate.as_object().unwrap();
        assert!(validate_event_fields("navigate", navigate, "test").is_err());

        let selector_click = json!({"kind":"click","selector":"#save","x":1});
        let selector_click = selector_click.as_object().unwrap();
        assert!(validate_event_fields("click", selector_click, "test").is_err());

        let coordinate_click = json!({"kind":"click","x":1,"y":2,"button":"left"});
        let coordinate_click = coordinate_click.as_object().unwrap();
        assert!(validate_event_fields("click", coordinate_click, "test").is_ok());

        let oversized = json!({"text": "x".repeat(MAX_INPUT_TEXT_BYTES + 1)});
        assert!(bounded_string(
            oversized.as_object().unwrap(),
            "text",
            MAX_INPUT_TEXT_BYTES,
            "test",
        )
        .is_err());
    }
}
