import unittest

from easynet_sdk import ErrorCode, HealthClient, SDKError, is_code


class StaticHealthTransport:
    def __init__(self, payload: bytes):
        self.payload = payload

    def runtime_health(self) -> bytes:
        return self.payload


class FailingHealthTransport:
    def runtime_health(self) -> bytes:
        raise RuntimeError("daemon unavailable")


class HealthClientTests(unittest.TestCase):
    def test_runtime_health_decodes_ready_fixture(self) -> None:
        client = HealthClient(
            StaticHealthTransport(
                b"""{
                    "api_ready": true,
                    "daemon_ready": true,
                    "invocation_ready": true,
                    "directory_ready": true,
                    "trust_ready": true,
                    "runtime_ready": true,
                    "version": "0.1.0",
                    "abi_version": 4,
                    "mismatch": null,
                    "diagnostics": []
                }"""
            )
        )

        health = client.runtime_health()

        self.assertTrue(health.api_alive())
        self.assertTrue(health.ready())
        self.assertEqual(health.abi_version, 4)

    def test_runtime_health_exposes_control_only_state(self) -> None:
        client = HealthClient(
            StaticHealthTransport(
                b"""{
                    "api_ready": true,
                    "daemon_ready": true,
                    "invocation_ready": false,
                    "directory_ready": true,
                    "trust_ready": true,
                    "runtime_ready": false,
                    "diagnostics": ["invocation endpoint unavailable"]
                }"""
            )
        )

        health = client.runtime_health()

        self.assertTrue(health.api_alive())
        self.assertFalse(health.ready())
        self.assertFalse(health.invocation_ready)

    def test_runtime_health_wraps_transport_failure(self) -> None:
        client = HealthClient(FailingHealthTransport())

        with self.assertRaises(SDKError) as caught:
            client.runtime_health()

        self.assertTrue(is_code(caught.exception, ErrorCode.TRANSPORT))
        self.assertIsInstance(caught.exception.cause, RuntimeError)

    def test_runtime_health_rejects_malformed_payload(self) -> None:
        client = HealthClient(
            StaticHealthTransport(b'{"api_ready": true, "runtime_ready": false}')
        )

        with self.assertRaises(SDKError) as caught:
            client.runtime_health()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_runtime_health_rejects_boolean_abi_version(self) -> None:
        client = HealthClient(
            StaticHealthTransport(
                b"""{
                    "api_ready": true,
                    "daemon_ready": true,
                    "invocation_ready": true,
                    "directory_ready": true,
                    "trust_ready": true,
                    "runtime_ready": true,
                    "abi_version": true
                }"""
            )
        )

        with self.assertRaises(SDKError) as caught:
            client.runtime_health()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))


if __name__ == "__main__":
    unittest.main()
