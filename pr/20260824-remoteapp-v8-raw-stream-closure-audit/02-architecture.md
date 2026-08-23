# Architecture

```text
Axon InvokeStreamChunk
  -> Runtime Core validation and lifecycle state machine
  -> canonical stream delivery
       -> ABI v7 JSON + payload_base64
       -> ABI v8 metadata JSON + borrowed raw payload
  -> SDK-owned callback copy + bounded queue
  -> product-neutral StreamEvent
  -> RemoteApp/EasyRemote consumer
```

ABI selection belongs to the SDK transport adapter. RemoteApp consumes the
product-neutral stream object and must not construct receipts, infer terminal
state, or call the raw C symbol directly.
