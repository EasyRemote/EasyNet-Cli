// EasyNet CLI — system.discuss.{create,post,subscribe} (PR-DISCUSS)
// ===================================================================
//
// File: src/runtime/system/discuss_ability.rs
// Description: Three abilities for hosting an asynchronous multi-
//              agent chat room over the IPC plane:
//
//   * `system.discuss.create`    (RPC) — spin up a new room.
//   * `system.discuss.post`      (RPC) — append one turn.
//   * `system.discuss.subscribe` (Stream) — read turns ≥ since_seq.
//
// Why three abilities, not one
// ----------------------------
// Create is a privileged op (allocates server state); post is
// per-turn and frequent; subscribe is long-lived. Splitting them
// into separate ability names lets a Client express the natural
// usage pattern over the dispatch layer without cramming a verb
// field into the args.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{LocalAbilityRegistry, StreamSource};
use crate::runtime::domain::{AgentId, RoomId};
use crate::runtime::execution::discuss::DiscussService;

pub const ABILITY_CREATE: &str = "discuss.create";
pub const ABILITY_POST: &str = "discuss.post";
pub const ABILITY_SUBSCRIBE: &str = "discuss.subscribe";

pub fn register(reg: &mut LocalAbilityRegistry, svc: Arc<DiscussService>) {
    let a = Arc::clone(&svc);
    reg.register_rpc(
        ABILITY_CREATE,
        Arc::new(move |args: Value| create_handler(&a, args)),
    );
    let b = Arc::clone(&svc);
    reg.register_rpc(
        ABILITY_POST,
        Arc::new(move |args: Value| post_handler(&b, args)),
    );
    reg.register_stream(
        ABILITY_SUBSCRIBE,
        Arc::new(move |args: Value| subscribe_handler(&svc, args)),
    );
}

fn create_handler(svc: &DiscussService, args: Value) -> anyhow::Result<Value> {
    let participants: Vec<String> = args
        .get("participants")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("discuss.create: `participants` (array of strings) required"))?
        .iter()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();
    let topic = args
        .get("topic")
        .and_then(Value::as_str)
        .map(String::from);
    let id = svc.create(participants, topic)?;
    Ok(json!({ "room_id": id.as_str() }))
}

fn post_handler(svc: &DiscussService, args: Value) -> anyhow::Result<Value> {
    let room_id = args
        .get("room_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("discuss.post: `room_id` required"))?;
    let speaker = args
        .get("speaker")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("discuss.post: `speaker` required"))?;
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("discuss.post: `message` required"))?;
    let payload = args.get("payload").cloned();
    let seq = svc.post(
        &RoomId::new(room_id),
        AgentId::new(speaker),
        message,
        payload,
    )?;
    Ok(json!({ "sequence": seq }))
}

fn subscribe_handler(svc: &DiscussService, args: Value) -> anyhow::Result<StreamSource> {
    let room_id = args
        .get("room_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("discuss.subscribe: `room_id` required"))?;
    let since = args.get("since_seq").and_then(Value::as_i64).unwrap_or(0);
    let room = RoomId::new(room_id);
    // Snapshot of past turns ≥ since_seq...
    let snapshot: Vec<Value> = svc
        .turns_from(&room, since)?
        .into_iter()
        .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
        .collect();
    // ...then live tail of new turns, relayed through a Value
    // broadcast so the IPC server can forward without knowing
    // the typed shape.
    let mut typed_rx = svc.subscribe_room(&room)?;
    let (json_tx, json_rx) = tokio::sync::broadcast::channel::<Value>(64);
    tokio::spawn(async move {
        loop {
            match typed_rx.recv().await {
                Ok(turn) => {
                    let v = serde_json::to_value(&turn).unwrap_or(Value::Null);
                    let _ = json_tx.send(v);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    Ok(StreamSource::SnapshotThenLive(snapshot, json_rx))
}

pub fn create_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["participants"],
        "properties": {
            "participants": {
                "type": "array",
                "items": {"type": "string"}
            },
            "topic": {"type": "string"}
        },
        "additionalProperties": false
    })
}

pub fn post_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["room_id", "speaker", "message"],
        "properties": {
            "room_id": {"type": "string"},
            "speaker": {"type": "string"},
            "message": {"type": "string"},
            "payload": {}
        },
        "additionalProperties": false
    })
}

pub fn subscribe_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["room_id"],
        "properties": {
            "room_id": {"type": "string"},
            "since_seq": {"type": "integer", "minimum": 0}
        },
        "additionalProperties": false
    })
}

pub fn create_description() -> &'static str {
    "Create a multi-agent discussion room. Returns a fresh `room_id` participants can post into."
}

pub fn post_description() -> &'static str {
    "Append one turn to a room. Returns the assigned `sequence`. The optional `payload` field carries structured data alongside the message text."
}

pub fn subscribe_description() -> &'static str {
    "Subscribe to a room's turn stream. Replays every turn ≥ `since_seq` (default 0). v1 returns a snapshot; live tail lands with PR-INVOCATION-EXEC-UNITY."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Arc<DiscussService> {
        Arc::new(DiscussService::new())
    }

    #[tokio::test]
    async fn create_post_subscribe_round_trip() {
        let svc = fresh();
        let resp = create_handler(
            &svc,
            json!({"participants": ["alice", "bob"], "topic": "ping pong"}),
        )
        .unwrap();
        let room = resp["room_id"].as_str().unwrap().to_string();

        post_handler(
            &svc,
            json!({
                "room_id": room,
                "speaker": "alice",
                "message": "hi"
            }),
        )
        .unwrap();
        post_handler(
            &svc,
            json!({
                "room_id": room,
                "speaker": "bob",
                "message": "hello"
            }),
        )
        .unwrap();

        let frames = subscribe_handler(&svc, json!({"room_id": room}))
            .unwrap()
            .into_snapshot();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["speaker"], "alice");
        assert_eq!(frames[1]["sequence"], 1);
    }

    #[tokio::test]
    async fn subscribe_since_seq_filters_prior_turns() {
        let svc = fresh();
        let room = create_handler(&svc, json!({"participants": ["a"]})).unwrap()
            ["room_id"]
            .as_str()
            .unwrap()
            .to_string();
        for i in 0..4 {
            post_handler(
                &svc,
                json!({"room_id": room, "speaker": "a", "message": format!("m{i}")}),
            )
            .unwrap();
        }
        let frames = subscribe_handler(&svc, json!({"room_id": room, "since_seq": 2}))
            .unwrap()
            .into_snapshot();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["sequence"], 2);
    }

    #[test]
    fn create_missing_participants_errors_clearly() {
        let svc = fresh();
        let err = create_handler(&svc, json!({})).unwrap_err();
        assert!(format!("{err}").contains("participants"));
    }

    #[test]
    fn post_unknown_room_returns_error() {
        let svc = fresh();
        let err = post_handler(
            &svc,
            json!({"room_id": "nope", "speaker": "a", "message": "hi"}),
        )
        .unwrap_err();
        assert!(format!("{err}").contains("does not exist"));
    }

    #[tokio::test]
    async fn post_payload_round_trips_through_subscribe() {
        let svc = fresh();
        let room = create_handler(&svc, json!({"participants": ["a"]})).unwrap()
            ["room_id"]
            .as_str()
            .unwrap()
            .to_string();
        post_handler(
            &svc,
            json!({
                "room_id": room,
                "speaker": "a",
                "message": "result",
                "payload": {"score": 0.92, "labels": ["ok"]}
            }),
        )
        .unwrap();
        let frames = subscribe_handler(&svc, json!({"room_id": room}))
            .unwrap()
            .into_snapshot();
        assert_eq!(frames[0]["payload"]["score"], 0.92);
    }
}
