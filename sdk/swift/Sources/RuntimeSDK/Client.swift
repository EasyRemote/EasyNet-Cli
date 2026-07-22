public struct FeatureSet: Sendable {
    public let abiVersion: Int
    public let sdkVersion: String
    public let profiles: [String: String]
    public let symbols: [String: Bool]
    public let protocolBridgeAvailable: Bool

    public init(
        abiVersion: Int,
        sdkVersion: String,
        profiles: [String: String] = [:],
        symbols: [String: Bool] = [:],
        protocolBridgeAvailable: Bool = false
    ) throws {
        guard abiVersion > 0 else {
            throw SDKError.validation("feature_discovery", "abiVersion must be positive")
        }
        guard !sdkVersion.isEmpty else {
            throw SDKError.validation("feature_discovery", "sdkVersion is required")
        }
        self.abiVersion = abiVersion
        self.sdkVersion = sdkVersion
        self.profiles = profiles
        self.symbols = symbols
        self.protocolBridgeAvailable = protocolBridgeAvailable
    }
}

public protocol DiscoveryTransport: AnyObject, Sendable {
    func featureDiscovery() async throws -> FeatureSet
    func close() async throws
}

public extension DiscoveryTransport {
    func close() async throws {}
}

public final class Client: @unchecked Sendable {
    private let transport: DiscoveryTransport
    private var closed = false

    public init(transport: DiscoveryTransport) {
        self.transport = transport
    }

    public func featureDiscovery() async throws -> FeatureSet {
        try requireOpen()
        return try await transport.featureDiscovery()
    }

    public func requireABI(_ expected: Int) async throws -> FeatureSet {
        let features = try await featureDiscovery()
        guard features.abiVersion == expected else {
            throw SDKError(
                code: .versionIncompatible,
                stage: "feature_discovery",
                message: "ABI version mismatch",
                details: ["expected": String(expected), "actual": String(features.abiVersion)]
            )
        }
        return features
    }

    public func close() async throws {
        guard !closed else {
            return
        }
        closed = true
        try await transport.close()
    }

    private func requireOpen() throws {
        if closed {
            throw SDKError.closed("client")
        }
    }
}
