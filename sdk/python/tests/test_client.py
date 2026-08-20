import unittest

from easynet_sdk import Client, ErrorCode, SDKError, is_code


class StaticTransport:
    def __init__(self, payload: bytes):
        self.payload = payload
        self.feature_calls = 0
        self.close_calls = 0
        self.close_error: BaseException | None = None

    def feature_discovery(self) -> bytes:
        self.feature_calls += 1
        return self.payload

    def close(self) -> None:
        self.close_calls += 1
        if self.close_error is not None:
            raise self.close_error


class FailingTransport:
    def feature_discovery(self) -> bytes:
        raise RuntimeError("daemon unavailable")

    def close(self) -> None:
        pass


class ClientTests(unittest.TestCase):
    def test_feature_discovery_decodes_runtime_core_facts(self) -> None:
        client = Client(
            StaticTransport(
                b"""{
                    "abi_version": 5,
                    "sdk_version": "0.91.30",
                    "profiles": {"runtime_core": "provider-backed"},
                    "symbols": {"runtime_health": true},
                    "axon_pb": true
                }"""
            )
        )

        features = client.feature_discovery()

        self.assertEqual(features.version().abi_version, 5)
        self.assertTrue(features.symbols["runtime_health"])

    def test_require_abi_returns_typed_version_mismatch(self) -> None:
        client = Client(StaticTransport(b'{"abi_version": 3, "sdk_version": "0.91.30"}'))

        with self.assertRaises(SDKError) as caught:
            client.require_abi(5)

        self.assertTrue(is_code(caught.exception, ErrorCode.VERSION_MISMATCH))
        self.assertIn("runtime ABI version", str(caught.exception))
        self.assertNotIn("daemon", str(caught.exception))

    def test_feature_discovery_wraps_transport_failure(self) -> None:
        client = Client(FailingTransport())

        with self.assertRaises(SDKError) as caught:
            client.feature_discovery()

        self.assertTrue(is_code(caught.exception, ErrorCode.ROUTE_UNAVAILABLE))
        self.assertIsInstance(caught.exception.cause, RuntimeError)

    def test_feature_discovery_rejects_malformed_payload(self) -> None:
        client = Client(StaticTransport(b'{"abi_version": true}'))

        with self.assertRaises(SDKError) as caught:
            client.feature_discovery()

        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))

    def test_require_abi_maps_zero_runtime_abi_to_version_mismatch(self) -> None:
        client = Client(StaticTransport(b'{"abi_version": 0, "sdk_version": "0.91.30"}'))

        with self.assertRaises(SDKError) as caught:
            client.require_abi(5)

        self.assertTrue(is_code(caught.exception, ErrorCode.VERSION_MISMATCH))
        self.assertIn("runtime ABI version", str(caught.exception))
        self.assertNotIn("daemon", str(caught.exception))

    def test_close_delegates_once_and_fails_closed(self) -> None:
        transport = StaticTransport(b'{"abi_version": 5, "sdk_version": "0.91.30"}')
        client = Client(transport)

        client.close()
        client.close()

        self.assertEqual(transport.close_calls, 1)
        with self.assertRaises(SDKError) as caught:
            client.feature_discovery()
        self.assertTrue(is_code(caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.feature_calls, 0)

    def test_close_failure_is_terminal(self) -> None:
        transport = StaticTransport(b'{"abi_version": 5, "sdk_version": "0.91.30"}')
        transport.close_error = RuntimeError("close failed")
        client = Client(transport)

        with self.assertRaises(SDKError) as close_caught:
            client.close()
        self.assertTrue(is_code(close_caught.exception, ErrorCode.ROUTE_UNAVAILABLE))
        self.assertIsInstance(close_caught.exception.cause, RuntimeError)

        with self.assertRaises(SDKError) as require_caught:
            client.require_abi(5)
        self.assertTrue(is_code(require_caught.exception, ErrorCode.INVALID_ARGUMENT))
        self.assertEqual(transport.feature_calls, 0)


if __name__ == "__main__":
    unittest.main()
