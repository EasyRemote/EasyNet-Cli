#pragma once

/*
 * Runtime C ABI v7 packaged by libeasynet_cli.
 *
 * The stable C surface owns runtime host lifecycle, generic Invocation lifecycle,
 * stream/bidi control, and runtime/error DTOs. Domain profile helpers belong
 * to language SDKs and are intentionally absent from this header.
 */

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define RUNTIME_ABI_VERSION 7u
#define RUNTIME_ABI_V8_EXTENSION_VERSION 8u

#define RUNTIME_OK 0
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

typedef uint64_t RuntimeHandle;
typedef uint64_t RuntimeHostHandle;
typedef uint64_t RuntimeInvocationStreamId;
typedef uint64_t RuntimeInvocationBidiId;
typedef uint64_t RuntimeInvocationBuilderId;
typedef uint64_t RuntimePreparedInvocationId;
typedef uint64_t RuntimeSignedInvocationId;
typedef uint64_t RuntimeInvocationHandleId;

/*
 * Callbacks run on library-owned threads. JSON pointers, v8 frame pointers,
 * and every RuntimeBytesViewV8 member are borrowed for the duration of the
 * call and must be copied or consumed before the callback returns.
 * `close_send` is a half-close; cancel/close/shutdown are terminal actions.
 */
typedef void (*RuntimeInvocationStreamCallback)(
    void *user_data,
    const char *chunk_json
);

#define RUNTIME_STREAM_FRAME_V8_ABI_VERSION 8u

#define RUNTIME_STREAM_FRAME_V8_KIND_DATA 1u
#define RUNTIME_STREAM_FRAME_V8_KIND_TERMINAL 2u
#define RUNTIME_STREAM_FRAME_V8_KIND_ERROR 3u
#define RUNTIME_STREAM_FRAME_V8_KIND_CANCELLED 4u
#define RUNTIME_STREAM_FRAME_V8_KIND_TIMEOUT 5u
#define RUNTIME_STREAM_FRAME_V8_KIND_RECEIPT_VERIFICATION_ERROR 6u

#define RUNTIME_STREAM_FRAME_V8_STATE_ACCEPTED 1u
#define RUNTIME_STREAM_FRAME_V8_STATE_ADMITTED 2u
#define RUNTIME_STREAM_FRAME_V8_STATE_DISPATCHED 3u
#define RUNTIME_STREAM_FRAME_V8_STATE_RUNNING 4u
#define RUNTIME_STREAM_FRAME_V8_STATE_COMPLETED 5u
#define RUNTIME_STREAM_FRAME_V8_STATE_FAILED 6u
#define RUNTIME_STREAM_FRAME_V8_STATE_TIMED_OUT 7u
#define RUNTIME_STREAM_FRAME_V8_STATE_CANCELLED 8u

#define RUNTIME_STREAM_FRAME_V8_FLAG_TERMINAL (1u << 0)
#define RUNTIME_STREAM_FRAME_V8_FLAG_TRANSPORT_TERMINAL (1u << 1)
#define RUNTIME_STREAM_FRAME_V8_FLAG_HAS_PAYLOAD (1u << 2)
#define RUNTIME_STREAM_FRAME_V8_FLAG_HAS_CONTENT_TYPE (1u << 3)
#define RUNTIME_STREAM_FRAME_V8_FLAG_HAS_ADMISSION_RECEIPT (1u << 4)
#define RUNTIME_STREAM_FRAME_V8_FLAG_HAS_TERMINAL_RECEIPT (1u << 5)
#define RUNTIME_STREAM_FRAME_V8_FLAG_HAS_ERROR (1u << 6)

typedef struct RuntimeBytesViewV8 {
    const uint8_t *data;
    size_t len;
} RuntimeBytesViewV8;

typedef struct RuntimeInvocationStreamFrameV8 {
    uint32_t struct_size;
    uint16_t abi_version;
    uint8_t kind;
    uint8_t state;
    uint32_t flags;
    uint64_t sequence;
    uint64_t elapsed_ms;
    RuntimeBytesViewV8 payload_content_type;
    RuntimeBytesViewV8 payload;
    RuntimeBytesViewV8 admission_receipt_json;
    RuntimeBytesViewV8 terminal_receipt_json;
    RuntimeBytesViewV8 error_json;
} RuntimeInvocationStreamFrameV8;

typedef void (*RuntimeInvocationStreamV8Callback)(
    void *user_data,
    const RuntimeInvocationStreamFrameV8 *frame
);

typedef void (*RuntimeInvocationBidiCallback)(
    void *user_data,
    const char *frame_json
);

uint32_t runtime_abi_version(void);
int32_t runtime_feature_discovery(char **out_features_json);
int32_t runtime_last_error_json(char **out_error_json);
int32_t runtime_error_json(
    int32_t code,
    const char *message,
    char **out_error_json
);
void runtime_string_free(char *s);

int32_t runtime_init(
    const char *control_json_path,
    RuntimeHandle *out_handle
);
int32_t runtime_shutdown(RuntimeHandle handle);

int32_t runtime_host_start(
    const char *config_json,
    RuntimeHostHandle *out_host_handle
);
int32_t runtime_host_attach(
    const char *options_json,
    RuntimeHostHandle *out_host_handle
);
int32_t runtime_host_discover(
    const char *options_json,
    char **out_discovery_json
);
int32_t runtime_host_stop(RuntimeHostHandle handle);
int32_t runtime_host_detach(RuntimeHostHandle handle);
int32_t runtime_host_status(
    RuntimeHostHandle handle,
    char **out_status_json
);
int32_t runtime_host_endpoints(
    RuntimeHostHandle handle,
    char **out_endpoints_json
);
int32_t runtime_host_invocation_endpoint(
    RuntimeHostHandle handle,
    char **out_endpoint
);
int32_t runtime_host_open_client(
    RuntimeHostHandle host_handle,
    RuntimeHandle *out_handle
);

int32_t runtime_health(
    RuntimeHandle handle,
    char **out_health_json
);
int32_t runtime_diagnostics(
    RuntimeHandle handle,
    char **out_diagnostics_json
);
int32_t runtime_resolve_descriptor_ref(
    RuntimeHandle handle,
    const char *request_json,
    char **out_descriptor_json
);
int32_t runtime_governance_read(
    RuntimeHandle handle,
    const char *invocation_json,
    char **out_result_json
);

int32_t runtime_invocation_invoke(
    RuntimeHandle handle,
    const char *invocation_json,
    char **out_receipt_json
);

int32_t runtime_invocation_builder_new(
    RuntimeInvocationBuilderId *out_builder_id
);
int32_t runtime_invocation_builder_set_caller(
    RuntimeInvocationBuilderId builder_id,
    const char *caller_ura
);
int32_t runtime_invocation_builder_set_callee(
    RuntimeInvocationBuilderId builder_id,
    const char *callee_ura
);
int32_t runtime_invocation_builder_set_descriptor_ref(
    RuntimeInvocationBuilderId builder_id,
    const char *descriptor_ref
);
int32_t runtime_invocation_builder_set_subject(
    RuntimeInvocationBuilderId builder_id,
    const char *subject_ura
);
int32_t runtime_invocation_builder_set_nonce_base64(
    RuntimeInvocationBuilderId builder_id,
    const char *nonce_base64
);
int32_t runtime_invocation_builder_set_causal_context_json(
    RuntimeInvocationBuilderId builder_id,
    const char *causal_context_json
);
int32_t runtime_invocation_builder_set_args_json(
    RuntimeInvocationBuilderId builder_id,
    const char *args_json
);
int32_t runtime_invocation_builder_set_arguments_base64(
    RuntimeInvocationBuilderId builder_id,
    const char *arguments_base64,
    const char *content_type
);
int32_t runtime_invocation_builder_set_metadata_json(
    RuntimeInvocationBuilderId builder_id,
    const char *metadata_json
);
int32_t runtime_invocation_builder_set_timeout_seconds(
    RuntimeInvocationBuilderId builder_id,
    uint32_t timeout_seconds
);
int32_t runtime_invocation_builder_set_idempotency_key(
    RuntimeInvocationBuilderId builder_id,
    const char *idempotency_key
);
int32_t runtime_invocation_builder_set_caller_signature_json(
    RuntimeInvocationBuilderId builder_id,
    const char *signature_json
);
int32_t runtime_invocation_builder_inspect(
    RuntimeInvocationBuilderId builder_id,
    char **out_invocation_json
);
int32_t runtime_invocation_builder_build(
    RuntimeInvocationBuilderId builder_id,
    char **out_invocation_json
);
int32_t runtime_invocation_builder_prepare(
    RuntimeHandle handle,
    RuntimeInvocationBuilderId builder_id,
    const char *options_json,
    RuntimePreparedInvocationId *out_prepared_id,
    char **out_prepared_json
);
int32_t runtime_invocation_builder_free(
    RuntimeInvocationBuilderId builder_id
);

int32_t runtime_invocation_prepare(
    RuntimeHandle handle,
    const char *invocation_json,
    const char *options_json,
    RuntimePreparedInvocationId *out_prepared_id,
    char **out_prepared_json
);
int32_t runtime_invocation_sign_prepared(
    RuntimePreparedInvocationId prepared_id,
    const char *signature_json,
    RuntimeSignedInvocationId *out_signed_id,
    char **out_signed_json
);
int32_t runtime_invocation_sign_prepared_local(
    RuntimePreparedInvocationId prepared_id,
    RuntimeSignedInvocationId *out_signed_id,
    char **out_signed_json
);
int32_t runtime_invocation_submit_signed_handle(
    RuntimeHandle handle,
    RuntimeSignedInvocationId signed_id,
    RuntimeInvocationHandleId *out_invocation_handle_id,
    char **out_submitted_json
);
int32_t runtime_invocation_handle_await(
    RuntimeHandle handle,
    RuntimeInvocationHandleId invocation_handle_id,
    char **out_result_json
);
int32_t runtime_invocation_handle_cancel(
    RuntimeHandle handle,
    RuntimeInvocationHandleId invocation_handle_id,
    const char *reason_json,
    char **out_cancel_json
);
int32_t runtime_invocation_handle_events(
    RuntimeHandle handle,
    RuntimeInvocationHandleId invocation_handle_id,
    char **out_events_json
);
int32_t runtime_invocation_handle_free(
    RuntimeHandle handle,
    RuntimeInvocationHandleId invocation_handle_id
);
int32_t runtime_prepared_invocation_free(
    RuntimePreparedInvocationId prepared_id
);
int32_t runtime_signed_invocation_free(
    RuntimeSignedInvocationId signed_id
);

int32_t runtime_invocation_stream_open(
    RuntimeHandle handle,
    const char *invocation_json,
    RuntimeInvocationStreamCallback on_chunk,
    void *user_data,
    RuntimeInvocationStreamId *out_stream_id
);
int32_t runtime_invocation_stream_open_v8(
    RuntimeHandle handle,
    const char *invocation_json,
    RuntimeInvocationStreamV8Callback on_chunk,
    void *user_data,
    RuntimeInvocationStreamId *out_stream_id
);
int32_t runtime_invocation_stream_cancel(
    RuntimeHandle handle,
    RuntimeInvocationStreamId stream_id
);
int32_t runtime_invocation_stream_close(
    RuntimeHandle handle,
    RuntimeInvocationStreamId stream_id
);

int32_t runtime_invocation_bidi_open(
    RuntimeHandle handle,
    const char *invocation_json,
    RuntimeInvocationBidiCallback on_frame,
    void *user_data,
    RuntimeInvocationBidiId *out_bidi_id
);
int32_t runtime_invocation_bidi_send(
    RuntimeHandle handle,
    RuntimeInvocationBidiId bidi_id,
    const char *frame_json
);
int32_t runtime_invocation_bidi_close_send(
    RuntimeHandle handle,
    RuntimeInvocationBidiId bidi_id
);
int32_t runtime_invocation_bidi_close(
    RuntimeHandle handle,
    RuntimeInvocationBidiId bidi_id
);
int32_t runtime_invocation_bidi_cancel(
    RuntimeHandle handle,
    RuntimeInvocationBidiId bidi_id
);

#ifdef __cplusplus
}
#endif
