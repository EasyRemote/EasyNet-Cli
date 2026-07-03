import unittest

from easynet_sdk import Client, ErrorCode, SDKError, is_code


class StaticTransport:
    def __init__(self, payload: bytes):
        self.payload = payload

    def feature_discovery(self) -> bytes:
        return self.payload


class FailingTransport:
    def feature_discovery(self) -> bytes:
        raise RuntimeError("daemon unavailable")


class ClientTests(unittest.TestCase):
    def test_feature_discovery_decodes_runtime_core_facts(self) -> None:
        client = Client(
            StaticTransport(
                b"""{
                    "abi_version": 4,
                    "sdk_version": "0.91.30",
                    "profiles": {"runtime_core": "partial"},
                    "symbols": {"runtime_health": true},
                    "axon_pb": true
                }"""
            )
        )

        features = client.feature_discovery()

        self.assertEqual(features.version().abi_version, 4)
        self.assertTrue(features.symbols["runtime_health"])

    def test_require_abi_returns_typed_version_mismatch(self) -> None:
        client = Client(StaticTransport(b'{"abi_version": 3, "sdk_version": "0.91.30"}'))

        with self.assertRaises(SDKError) as caught:
            client.require_abi(4)

        self.assertTrue(is_code(caught.exception, ErrorCode.VERSION_INCOMPATIBLE))

    def test_feature_discovery_wraps_transport_failure(self) -> None:
        client = Client(FailingTransport())

        with self.assertRaises(SDKError) as caught:
            client.feature_discovery()

        self.assertTrue(is_code(caught.exception, ErrorCode.TRANSPORT))
        self.assertIsInstance(caught.exception.cause, RuntimeError)

    def test_feature_discovery_rejects_malformed_payload(self) -> None:
        client = Client(StaticTransport(b'{"abi_version": 0}'))

        with self.assertRaises(SDKError) as caught:
            client.feature_discovery()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))


if __name__ == "__main__":
    unittest.main()
