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
 *   - easynet_invocation_stream_close
 *   - easynet_invocation_bidi_close
 *   - easynet_invocation_bidi_cancel
 *   - easynet_shutdown on the owning EasynetHandle
 * `easynet_invocation_bidi_close_send` is not terminal; it only
 * half-closes the local send side.
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
int32_t easynet_last_error_json(char **out_error_json);
int32_t easynet_error_json(
    int32_t code,
    const char *message,
    char **out_error_json
);
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

int32_t easynet_invocation_stream_close(
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

int32_t easynet_invocation_bidi_close_send(
    EasynetHandle handle,
    EasynetInvocationBidiId bidi_id
);

int32_t easynet_invocation_bidi_close(
    EasynetHandle handle,
    EasynetInvocationBidiId bidi_id
);

int32_t easynet_invocation_bidi_cancel(
    EasynetHandle handle,
    EasynetInvocationBidiId bidi_id
);

int32_t easynet_identity_project_ura(
    EasynetHandle handle,
    const char *ura,
    char **out_identity_json
);

int32_t easynet_identity_build_ura(
    EasynetHandle handle,
    const char *request_json,
    char **out_identity_json
);

int32_t easynet_identity_project_descriptor_ref(
    EasynetHandle handle,
    const char *descriptor_ref,
    char **out_descriptor_json
);

int32_t easynet_identity_build_descriptor_ref(
    EasynetHandle handle,
    const char *request_json,
    char **out_descriptor_json
);

int32_t easynet_identity_build_register_signing_key_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_identity_build_list_signing_keys_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_identity_build_revoke_signing_key_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_directory_build_list_devices_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_directory_build_list_agents_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_directory_build_list_abilities_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_directory_build_resolve_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_directory_project_device_page(
    EasynetHandle handle,
    const char *devices_json,
    char **out_page_json
);

int32_t easynet_directory_project_agent_page(
    EasynetHandle handle,
    const char *agents_json,
    char **out_page_json
);

int32_t easynet_directory_project_ability_page(
    EasynetHandle handle,
    const char *abilities_json,
    char **out_page_json
);

int32_t easynet_directory_project_resolved_ref(
    EasynetHandle handle,
    const char *answer_json,
    char **out_resolved_ref_json
);

int32_t easynet_receipt_build_fetch_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_receipt_project(
    EasynetHandle handle,
    const char *receipt_json,
    char **out_summary_json
);

int32_t easynet_receipt_verify(
    EasynetHandle handle,
    const char *receipt_json,
    char **out_verification_json
);

int32_t easynet_receipt_verify_chain(
    EasynetHandle handle,
    const char *request_json,
    char **out_verification_json
);

int32_t easynet_receipt_causal_ref(
    EasynetHandle handle,
    const char *receipt_json,
    char **out_causal_ref_json
);

int32_t easynet_host_binding_build(
    EasynetHandle handle,
    const char *request_json,
    char **out_binding_json
);

int32_t easynet_host_binding_decode_request(
    EasynetHandle handle,
    const char *envelope_json,
    char **out_request_json
);

int32_t easynet_host_binding_encode_item(
    EasynetHandle handle,
    const char *item_json,
    char **out_frame_json
);

int32_t easynet_host_binding_encode_error(
    EasynetHandle handle,
    const char *error_json,
    char **out_frame_json
);

int32_t easynet_host_binding_encode_terminal(
    EasynetHandle handle,
    const char *terminal_json,
    char **out_frame_json
);

int32_t easynet_host_binding_fold_output_hash(
    EasynetHandle handle,
    const char *fold_json,
    char **out_state_json
);

int32_t easynet_publication_build_resource_ref(
    EasynetHandle handle,
    const char *request_json,
    char **out_resource_ref_json
);

int32_t easynet_publication_validate_package(
    EasynetHandle handle,
    const char *request_json,
    char **out_validation_json
);

int32_t easynet_publication_build_deploy_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_publication_project_deploy_result(
    EasynetHandle handle,
    const char *result_json,
    char **out_result_json
);

int32_t easynet_publication_build_list_abilities_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_publication_project_ability_page(
    EasynetHandle handle,
    const char *page_json,
    char **out_page_json
);

int32_t easynet_publication_build_show_ability_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_publication_project_ability_record(
    EasynetHandle handle,
    const char *record_json,
    char **out_ability_json
);

int32_t easynet_publication_build_unpublish_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_publication_project_unpublish_result(
    EasynetHandle handle,
    const char *result_json,
    char **out_result_json
);

int32_t easynet_mission_build_run_eal_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_mission_build_run_file_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_mission_build_track_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_mission_build_cancel_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_mission_project_status(
    EasynetHandle handle,
    const char *status_json,
    char **out_status_json
);

int32_t easynet_mission_project_events(
    EasynetHandle handle,
    const char *events_json,
    char **out_page_json
);

int32_t easynet_events_build_directory_subscription_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_events_project_directory_event(
    EasynetHandle handle,
    const char *event_json,
    char **out_event_json
);

int32_t easynet_events_project_terminal(
    EasynetHandle handle,
    const char *terminal_json,
    char **out_event_json
);

int32_t easynet_events_project_drop_report(
    EasynetHandle handle,
    const char *drop_json,
    char **out_event_json
);

int32_t easynet_admin_build_agent_list_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_admin_build_agent_start_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_admin_build_agent_stop_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_admin_build_agent_refresh_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_admin_build_session_list_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_admin_project_gateway_status(
    EasynetHandle handle,
    const char *status_json,
    char **out_status_json
);

int32_t easynet_admin_project_agent_records(
    EasynetHandle handle,
    const char *agents_json,
    char **out_agents_json
);

int32_t easynet_admin_project_agent_lifecycle_result(
    EasynetHandle handle,
    const char *result_json,
    char **out_result_json
);

int32_t easynet_surface_build_list_pages_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_surface_build_create_page_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_surface_build_delete_page_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_surface_build_manifest_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_surface_project_page_record(
    EasynetHandle handle,
    const char *page_json,
    char **out_page_json
);

int32_t easynet_surface_project_page_page(
    EasynetHandle handle,
    const char *pages_json,
    char **out_page_json
);

int32_t easynet_surface_project_manifest(
    EasynetHandle handle,
    const char *page_json,
    char **out_manifest_json
);

int32_t easynet_surface_project_public_page_ref(
    EasynetHandle handle,
    const char *page_json,
    char **out_ref_json
);

int32_t easynet_surface_project_mutation_result(
    EasynetHandle handle,
    const char *result_json,
    char **out_result_json
);

int32_t easynet_compatibility_build_list_models_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_compatibility_build_chat_completion_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_compatibility_build_stream_chat_completion_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_compatibility_build_file_upload_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_compatibility_build_file_retrieve_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_compatibility_build_file_delete_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_compatibility_project_model_page(
    EasynetHandle handle,
    const char *models_json,
    char **out_models_json
);

int32_t easynet_compatibility_project_chat_completion(
    EasynetHandle handle,
    const char *completion_json,
    char **out_completion_json
);

int32_t easynet_compatibility_project_chat_stream(
    EasynetHandle handle,
    const char *stream_json,
    char **out_stream_json
);

int32_t easynet_compatibility_project_file_upload(
    EasynetHandle handle,
    const char *file_json,
    char **out_file_json
);

int32_t easynet_compatibility_project_file(
    EasynetHandle handle,
    const char *file_json,
    char **out_file_json
);

int32_t easynet_compatibility_project_file_delete_result(
    EasynetHandle handle,
    const char *result_json,
    char **out_result_json
);

int32_t easynet_wrappers_build_file_transfer_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_wrappers_build_terminal_session_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_wrappers_build_remote_desktop_session_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_wrappers_build_browser_session_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_wrappers_build_media_session_invocation(
    EasynetHandle handle,
    const char *request_json,
    char **out_invocation_json
);

int32_t easynet_wrappers_project_file_record(
    EasynetHandle handle,
    const char *file_json,
    char **out_file_json
);

int32_t easynet_wrappers_project_terminal_session(
    EasynetHandle handle,
    const char *session_json,
    char **out_session_json
);

int32_t easynet_wrappers_project_remote_desktop_session(
    EasynetHandle handle,
    const char *session_json,
    char **out_session_json
);

int32_t easynet_wrappers_project_browser_session(
    EasynetHandle handle,
    const char *session_json,
    char **out_session_json
);

int32_t easynet_wrappers_project_media_session(
    EasynetHandle handle,
    const char *session_json,
    char **out_session_json
);

#ifdef __cplusplus
}
#endif
