# Architecture

The SPEC places AbilityDescriptor and DescriptorRef projection under the SDK runtime model. The SDK cannot import raw Axon Go helpers because the public SDK import-boundary forbids protocol/runtime packages. Therefore this slice adds a pure SDK projection over schemaless descriptor maps.

Layering:

- Axon owns canonical protocol semantics and daemon-side descriptor emission.
- EasyNet-Cli SDK owns generic descriptor projection DTOs for consumers.
- Products map SDK projection metadata into their own UI/API DTOs.

The Go and Python SDKs get the same projection concept to avoid language-specific architectural drift.
