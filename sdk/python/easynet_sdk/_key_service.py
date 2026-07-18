"""REQ-LANG-5 compatibility exports for the EasyNet key-service provider."""

from .providers.easynet.key_service import (
    KEY_SERVICE_PROTOCOL_VERSION,
    MAX_KEY_SERVICE_CANONICAL_BYTES,
    MAX_KEY_SERVICE_FRAME_BYTES,
    KeyServiceClient,
    decode_base64_field,
    decode_base64_value,
    invalid_key_service_input,
    invalid_key_service_payload,
    key_service_rejection,
    reject_private_response_fields,
    require_response_shape,
    require_result,
    required_response_bool,
    required_response_i64,
    required_response_string,
)

__all__ = [
    "KEY_SERVICE_PROTOCOL_VERSION",
    "MAX_KEY_SERVICE_CANONICAL_BYTES",
    "MAX_KEY_SERVICE_FRAME_BYTES",
    "KeyServiceClient",
    "decode_base64_field",
    "decode_base64_value",
    "invalid_key_service_input",
    "invalid_key_service_payload",
    "key_service_rejection",
    "reject_private_response_fields",
    "require_response_shape",
    "require_result",
    "required_response_bool",
    "required_response_i64",
    "required_response_string",
]
