#pragma once

/*
 * EasyNet CLI C ABI v4.
 *
 * This header is the binding-facing contract for libeasynet_cli.
 * The Rust sources in src/ffi own the implementation; repository
 * checks keep this file in sync with exported symbols, ABI version,
 * and error-code semantics.
 */

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define EASYNET_ABI_VERSION 4u

#define EASYNET_OK 0
#define ERR_GENERIC 1
#define ERR_NULL_POINTER 2
#define ERR_INVALID_UTF8 3
#define ERR_INVALID_HANDLE 4
#define ERR_NOT_INITIALIZED 5
#define ERR_ALREADY_INIT 6
#define ERR_DAEMON_DOWN 7
#define ERR_VERSION_INCOMPATIBLE 8
#define ERR_ABILITY_FAILED 9
#define ERR_NOT_IMPLEMENTED 10
#define ERR_INVALID_ARG 11
#define ERR_PERMISSION_DENIED 12
#define ERR_NOT_FOUND 13
#define ERR_CANCELLED 14
#define ERR_PROTOCOL 15
#define ERR_TIMEOUT 16

typedef uint64_t EasynetHandle;
typedef uint64_t EasynetDaemonHandle;
typedef uint64_t EasynetInvocationStreamId;
typedef uint64_t EasynetInvocationBidiId;
typedef uint64_t EasynetInvocationBuilderId;
typedef uint64_t EasynetPreparedInvocationId;
typedef uint64_t EasynetSignedInvocationId;
typedef uint64_t EasynetInvocationHandleId;

/*
 * Stream and bidi callbacks are invoked on libeasynet_cli-owned
 * background threads, not necessarily on the thread that opened the
 * stream/session. Bindings must treat callbacks as concurrent with
 * cancellation and shutdown.
 *
 * `chunk_json` / `frame_json` are borrowed only for the duration of
 * the callback. Copy the string before returning if it must outlive
 * the call.
 *
 * `user_data` is never inspected by Rust. It must remain valid until
 * the callback has returned after one of these terminal actions:
 *   - easynet_invocation_stream_cancel
 *   - easynet_invocation_bidi_close
 *   - easynet_invocation_bidi_cancel
 *   - easynet_shutdown on the owning EasynetHandle
 *
 * A callback must not unwind across the C ABI. Language bindings that
 * can throw exceptions must catch them inside the callback shim.
 */
typedef void (*EasynetInvocationStreamCallback)(
    void *user_data,
    const char *chunk_json
);

typedef void (*EasynetInvocationBidiCallback)(
    void *user_data,
    const char *frame_json
);

uint32_t easynet_abi_version(void);
int32_t easynet_feature_discovery(char **out_features_json);
const char *easynet_last_error(void);
void easynet_string_free(char *s);

int32_t easynet_init(
    const char *control_json_path,
    EasynetHandle *out_handle
);

int32_t easynet_shutdown(EasynetHandle handle);

int32_t easynet_daemon_start(
    const char *config_json,
    EasynetDaemonHandle *out_daemon_handle
);

int32_t easynet_daemon_attach(
    const char *options_json,
    EasynetDaemonHandle *out_daemon_handle
);

int32_t easynet_daemon_discover(
    const char *options_json,
    char **out_discovery_json
);

int32_t easynet_daemon_stop(EasynetDaemonHandle handle);

int32_t easynet_daemon_detach(EasynetDaemonHandle handle);

int32_t easynet_daemon_status(
    EasynetDaemonHandle handle,
    char **out_status_json
);

int32_t easynet_daemon_endpoints(
    EasynetDaemonHandle handle,
    char **out_endpoints_json
);

int32_t easynet_daemon_invocation_endpoint(
    EasynetDaemonHandle handle,
    char **out_endpoint
);

int32_t easynet_daemon_open_client(
    EasynetDaemonHandle daemon_handle,
    EasynetHandle *out_handle
);

int32_t easynet_invocation_invoke(
    EasynetHandle handle,
    const char *invocation_json,
    char **out_receipt_json
);

int32_t easynet_runtime_health(
    EasynetHandle handle,
    char **out_health_json
);

int32_t easynet_invocation_builder_new(
    EasynetInvocationBuilderId *out_builder_id
);

int32_t easynet_invocation_builder_set_caller(
    EasynetInvocationBuilderId builder_id,
    const char *caller_ura
);

int32_t easynet_invocation_builder_set_callee(
    EasynetInvocationBuilderId builder_id,
    const char *callee_ura
);

int32_t easynet_invocation_builder_set_descriptor_ref(
    EasynetInvocationBuilderId builder_id,
    const char *descriptor_ref
);

int32_t easynet_invocation_builder_set_subject(
    EasynetInvocationBuilderId builder_id,
    const char *subject_ura
);

int32_t easynet_invocation_builder_set_nonce_base64(
    EasynetInvocationBuilderId builder_id,
    const char *nonce_base64
);

int32_t easynet_invocation_builder_set_causal_context_json(
    EasynetInvocationBuilderId builder_id,
    const char *causal_context_json
);

int32_t easynet_invocation_builder_set_args_json(
    EasynetInvocationBuilderId builder_id,
    const char *args_json
);

int32_t easynet_invocation_builder_set_arguments_base64(
    EasynetInvocationBuilderId builder_id,
    const char *arguments_base64,
    const char *content_type
);

int32_t easynet_invocation_builder_set_metadata_json(
    EasynetInvocationBuilderId builder_id,
    const char *metadata_json
);

int32_t easynet_invocation_builder_set_timeout_seconds(
    EasynetInvocationBuilderId builder_id,
    uint32_t timeout_seconds
);

int32_t easynet_invocation_builder_set_idempotency_key(
    EasynetInvocationBuilderId builder_id,
    const char *idempotency_key
);

int32_t easynet_invocation_builder_set_caller_signature_json(
    EasynetInvocationBuilderId builder_id,
    const char *signature_json
);

int32_t easynet_invocation_builder_inspect(
    EasynetInvocationBuilderId builder_id,
    char **out_invocation_json
);

int32_t easynet_invocation_builder_build(
    EasynetInvocationBuilderId builder_id,
    char **out_invocation_json
);

int32_t easynet_invocation_builder_prepare(
    EasynetHandle handle,
    EasynetInvocationBuilderId builder_id,
    const char *options_json,
    EasynetPreparedInvocationId *out_prepared_id,
    char **out_prepared_json
);

int32_t easynet_invocation_builder_free(
    EasynetInvocationBuilderId builder_id
);

int32_t easynet_invocation_prepare(
    EasynetHandle handle,
    const char *invocation_json,
    const char *options_json,
    EasynetPreparedInvocationId *out_prepared_id,
    char **out_prepared_json
);

int32_t easynet_invocation_sign_prepared(
    EasynetPreparedInvocationId prepared_id,
    const char *signature_json,
    EasynetSignedInvocationId *out_signed_id,
    char **out_signed_json
);

int32_t easynet_invocation_submit_signed(
    EasynetHandle handle,
    EasynetSignedInvocationId signed_id,
    char **out_result_json
);

int32_t easynet_invocation_submit_signed_handle(
    EasynetHandle handle,
    EasynetSignedInvocationId signed_id,
    EasynetInvocationHandleId *out_invocation_handle_id,
    char **out_submitted_json
);

int32_t easynet_invocation_handle_await(
    EasynetHandle handle,
    EasynetInvocationHandleId invocation_handle_id,
    char **out_result_json
);

int32_t easynet_invocation_handle_cancel(
    EasynetHandle handle,
    EasynetInvocationHandleId invocation_handle_id,
    const char *reason_json,
    char **out_cancel_json
);

int32_t easynet_invocation_handle_events(
    EasynetHandle handle,
    EasynetInvocationHandleId invocation_handle_id,
    char **out_events_json
);

int32_t easynet_invocation_handle_free(
    EasynetHandle handle,
    EasynetInvocationHandleId invocation_handle_id
);

int32_t easynet_prepared_invocation_free(
    EasynetPreparedInvocationId prepared_id
);

int32_t easynet_signed_invocation_free(
    EasynetSignedInvocationId signed_id
);

int32_t easynet_invocation_stream_open(
    EasynetHandle handle,
    const char *invocation_json,
    EasynetInvocationStreamCallback on_chunk,
    void *user_data,
    EasynetInvocationStreamId *out_stream_id
);

int32_t easynet_invocation_stream_cancel(
    EasynetHandle handle,
    EasynetInvocationStreamId stream_id
);

int32_t easynet_invocation_bidi_open(
    EasynetHandle handle,
    const char *invocation_json,
    EasynetInvocationBidiCallback on_frame,
    void *user_data,
    EasynetInvocationBidiId *out_bidi_id
);

int32_t easynet_invocation_bidi_send(
    EasynetHandle handle,
    EasynetInvocationBidiId bidi_id,
    const char *frame_json
);

int32_t easynet_invocation_bidi_close(
    EasynetHandle handle,
    EasynetInvocationBidiId bidi_id
);

int32_t easynet_invocation_bidi_cancel(
    EasynetHandle handle,
    EasynetInvocationBidiId bidi_id
);

#ifdef __cplusplus
}
#endif
