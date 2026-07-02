// EasyNet Daemon — Session Frame Carrier Baseline (T0.4)
// ========================================================
//
// File: engineering/benches/session_frame_carrier.rs
// Description: Quantifies the cost of today's second invocation
//              carrier (to-be-fix.spec §A2 / F-004): session business
//              frames travel as serde-JSON `SessionDispatch` inside a
//              proto `BinaryChunk` inside `InvokeBidiDown` — every
//              frame pays a JSON encode + proto encode on send and a
//              proto decode + JSON decode on receive, and `args:
//              Vec<u8>` serializes as a JSON number array.
//
//              Three measured layers per payload size (1KB / 64KB):
//                json_codec        — SessionDispatch JSON encode/decode
//                carrier_roundtrip — the full double-parse wire path
//                canonical_proto   — InvokeRequest proto roundtrip,
//                                    i.e. the T2.1 target carrier
//
//              The carrier_roundtrip / canonical_proto ratio is the
//              number T2.1 cites as its expected win. Run with
//              `cargo bench --bench session_frame_carrier`; record
//              results in docs/bench/.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use prost::Message;

use easynet_axon::pb::axon::v1::{
    invoke_bidi_down::Payload as DownPayload, AgentIdentity, BinaryChunk, Envelope, InvokeBidiDown,
    InvokeRequest, SubjectIdentity,
};
use easynet_cli::daemon::invocation::invoke_remote_initiator::{
    SessionContentEnvelope, SessionDispatch, INVOKE_REMOTE_STREAM_ID,
};

const CALLER: &str = "easynet:///r/bench-realm/device/bench-caller";
const CALLEE: &str = "easynet:///r/bench-realm/agent/bench.worker";
const ABILITY: &str = "bench.worker.echo";

fn dispatch_frame(args: Vec<u8>) -> SessionDispatch {
    SessionDispatch::Dispatch {
        call_id: 42,
        callee_ura: Some(CALLEE.to_string()),
        subject_ura: Some(CALLER.to_string()),
        ability: ABILITY.to_string(),
        args,
        args_content_envelope: SessionContentEnvelope::plaintext_json(),
        metadata: HashMap::new(),
        origin_caller: None,
    }
}

fn invoke_request(args: Vec<u8>) -> InvokeRequest {
    InvokeRequest {
        envelope: Some(Envelope {
            caller: Some(AgentIdentity {
                ura: CALLER.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            callee: Some(AgentIdentity {
                ura: CALLEE.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            subject: Some(SubjectIdentity {
                ura: CALLER.to_string(),
                profile: "easynet-strict-v2".to_string(),
            }),
            invocation_nonce: vec![7; 16],
            ..Envelope::default()
        }),
        function_name: ABILITY.to_string(),
        arguments: args,
        ..InvokeRequest::default()
    }
}

/// Today's full send+receive path: JSON encode → BinaryChunk →
/// InvokeBidiDown proto encode → proto decode → JSON decode.
fn carrier_roundtrip(frame: &SessionDispatch) -> SessionDispatch {
    let json = serde_json::to_vec(frame).expect("encode SessionDispatch");
    let down = InvokeBidiDown {
        payload: Some(DownPayload::BinaryChunk(BinaryChunk {
            stream_id: INVOKE_REMOTE_STREAM_ID,
            data: json,
            ..BinaryChunk::default()
        })),
        ..InvokeBidiDown::default()
    };
    let wire = down.encode_to_vec();

    let decoded = InvokeBidiDown::decode(wire.as_slice()).expect("decode InvokeBidiDown");
    let Some(DownPayload::BinaryChunk(chunk)) = decoded.payload else {
        panic!("expected BinaryChunk payload");
    };
    serde_json::from_slice(&chunk.data).expect("decode SessionDispatch")
}

fn bench_carrier(c: &mut Criterion) {
    for (label, size) in [("1k", 1024_usize), ("64k", 64 * 1024)] {
        let args = vec![0xA5_u8; size];
        let frame = dispatch_frame(args.clone());
        let frame_json = serde_json::to_vec(&frame).expect("encode");
        let request = invoke_request(args.clone());
        let request_wire = request.encode_to_vec();

        let mut group = c.benchmark_group("session_frame_carrier");
        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("json_encode", label), &frame, |b, f| {
            b.iter(|| serde_json::to_vec(f).expect("encode"))
        });
        group.bench_with_input(
            BenchmarkId::new("json_decode", label),
            &frame_json,
            |b, j| b.iter(|| serde_json::from_slice::<SessionDispatch>(j).expect("decode")),
        );
        group.bench_with_input(
            BenchmarkId::new("carrier_roundtrip", label),
            &frame,
            |b, f| b.iter(|| carrier_roundtrip(f)),
        );
        // After-side of the T2.1 migration: the REAL carrier-v1 frame
        // (DispatchCall carrying the complete InvokeRequest inside
        // InvokeBidiDown) — what step 2d ships on the wire today for
        // v1 devices. Cite this against carrier_roundtrip in the
        // step-5 re-baseline.
        let v1_frame = easynet_axon::pb::axon::v1::InvokeBidiDown {
            payload: Some(DownPayload::DispatchCall(
                easynet_axon::pb::axon::v1::DispatchCall {
                    call_id: 42,
                    request: Some(request.clone()),
                    open_bidi: false,
                },
            )),
            ..Default::default()
        };
        group.bench_with_input(
            BenchmarkId::new("carrier_v1_roundtrip", label),
            &v1_frame,
            |b, f| {
                b.iter(|| {
                    let wire = f.encode_to_vec();
                    easynet_axon::pb::axon::v1::InvokeBidiDown::decode(wire.as_slice())
                        .expect("decode v1 frame")
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("canonical_proto_roundtrip", label),
            &request,
            |b, r| {
                b.iter(|| {
                    let wire = r.encode_to_vec();
                    InvokeRequest::decode(wire.as_slice()).expect("decode InvokeRequest")
                })
            },
        );

        // Wire-size comparison printed once per size so the report can
        // cite bytes alongside time. JSON inflates `args: Vec<u8>` into
        // a number array; proto carries it verbatim.
        eprintln!(
            "[wire-size {label}] session_json={}B canonical_proto={}B inflation={:.2}x",
            frame_json.len(),
            request_wire.len(),
            frame_json.len() as f64 / request_wire.len() as f64,
        );
        group.finish();
    }
}

criterion_group!(benches, bench_carrier);
criterion_main!(benches);
