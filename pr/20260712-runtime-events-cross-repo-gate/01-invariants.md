1. Runtime Events stay product-neutral in the SDK: stream kind, cursor, page and
   subscription draft are generic runtime concepts.
2. Backend may adapt product SSE/device/session use cases through the Go SDK
   event subscription client, but must not rebuild descriptor refs or
   Invocation lowering by hand.
3. EasyRemote product mission events remain downstream product workflow; this
   gate only proves product event consumers do not force SDK event model drift.
4. Passing this gate is adapter evidence, not final live event cutover evidence.
