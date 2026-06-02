// EasyNet CLI — hosted receipt SDK re-export.
//
// §A12 receipt model ownership lives in `easynet_axon::invocation::audit`.
// Keep this module path as a compatibility shim for existing CLI callers.

pub use easynet_axon::invocation::audit::{
    HostedAgentReceiptHeader, HostedReceiptError, SigningModel,
};
