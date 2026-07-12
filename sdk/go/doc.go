// Package easynet provides the Go binding for the canonical EasyNet runtime SDK.
//
// The package is product-neutral. EasyNet backend, EasyRemote and future
// products own their workflows, DTOs, account policy, HTTP routes and UI
// projections downstream; they lower those workflows through the generic
// runtime concepts exported here.
//
// Public APIs expose daemon lifecycle, Addressing, Invocation, signing,
// PrincipalLifecycle, Directory, receipts, runtime events, administration,
// health and typed errors. They do not expose Axon SDK packages, generated
// protobufs, C ABI handles, daemon internals or product profile clients.
package easynet
