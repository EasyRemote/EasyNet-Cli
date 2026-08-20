# Backend SDK-Only Import Ban Intent

Add an executable EasyNet backend SDK-only boundary gate for the daemon SDK
requirements.

The gate belongs in EasyNet-Cli because this repository owns the daemon SDK
facade and shared conformance assets. It scans an EasyNet backend Go tree and
flags production code that bypasses the public CLI Go SDK boundary by importing
Axon public packages, generated Axon protobufs, direct daemon transport
packages, C ABI/FFI markers, EasyRemote, or product runtime subprocess calls.

The script must be deterministic, runnable against a real sibling backend
checkout, and self-testable without that checkout.
