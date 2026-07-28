import Foundation

struct RuntimeAbilityProjection: Sendable, Equatable {
    private static let realmPrefix = "easynet:///r/"
    private static let runtimeGovernanceReadAbilities = [
        "meta.list_abilities",
        "invocation.history.list",
        "invocation.history.get",
        "invocation.history.path",
        "invocation.record.get",
        "invocation.trace.get",
    ]

    let abilityURA: String
    let publicName: String
    let intrinsicName: String

    private init(abilityURA: String, publicName: String, intrinsicName: String) {
        self.abilityURA = abilityURA
        self.publicName = publicName
        self.intrinsicName = intrinsicName
    }

    init(tuple: InvocationTuple) throws {
        self = try Self.fromDescriptorRef(calleeURA: tuple.callee, descriptorRef: tuple.descriptorRef)
    }

    static func runtimeGovernanceReadAbility(calleeURA: String, descriptorRef: String) throws -> String? {
        let ability = try fromDescriptorRef(calleeURA: calleeURA, descriptorRef: descriptorRef)
        return runtimeGovernanceReadAbility(ability.publicName) ?? runtimeGovernanceReadAbility(ability.intrinsicName)
    }

    static func runtimeGovernanceDescriptorProvider(forAbility ability: String) -> String {
        let clean = ability.trimmingCharacters(in: .whitespacesAndNewlines)
        if clean == "meta.list_abilities" || clean.hasSuffix(".meta.list_abilities") {
            return RuntimeDescriptorRefRequest.abilityDescriptorProvider
        }
        if runtimeGovernanceReadAbility(clean) != nil {
            return RuntimeDescriptorRefRequest.receiptHistoryProvider
        }
        return ""
    }

    static func authorityURAForRealmOf(_ ura: String) throws -> String {
        let clean = ura.trimmingCharacters(in: .whitespacesAndNewlines)
        guard clean.hasPrefix(realmPrefix) else {
            throw SDKError.validation("runtime", "callee_ura must be canonical for ability descriptor provider")
        }
        let rest = String(clean.dropFirst(realmPrefix.count))
        guard let slash = rest.firstIndex(of: "/"), slash != rest.startIndex else {
            throw SDKError.validation("runtime", "callee_ura must be canonical for ability descriptor provider")
        }
        let realm = String(rest[..<slash]).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !realm.isEmpty, !realm.contains("/") else {
            throw SDKError.validation("runtime", "callee_ura must be canonical for ability descriptor provider")
        }
        return "\(realmPrefix)\(realm)/authority"
    }

    static func abilityURAForDescriptorRef(_ descriptorRef: String) throws -> String {
        let projection = try descriptorAbilityProjection(descriptorRef)
        return projection.abilityURA
    }

    private static func fromDescriptorRef(calleeURA: String, descriptorRef: String) throws -> RuntimeAbilityProjection {
        let projection = try descriptorAbilityProjection(descriptorRef)
        return RuntimeAbilityProjection(
            abilityURA: projection.abilityURA,
            publicName: publicAbilityName(calleeURA: calleeURA, intrinsicName: projection.intrinsicName),
            intrinsicName: projection.intrinsicName
        )
    }

    private static func descriptorAbilityProjection(_ descriptorRef: String) throws -> AbilityDescriptorProjection {
        let clean = descriptorRef.trimmingCharacters(in: .whitespacesAndNewlines)
        let hash = clean.firstIndex(of: "#")
        let bang = clean.firstIndex(of: "!")
        var limit = clean.endIndex
        if let hash {
            limit = min(limit, hash)
        }
        if let bang {
            limit = min(limit, bang)
        }
        let withoutMode = String(clean[..<limit])
        let ability = if let version = withoutMode.lastIndex(of: "@") {
            String(withoutMode[..<version]).trimmingCharacters(in: .whitespacesAndNewlines)
        } else {
            withoutMode.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        let path = canonicalTopLevelPath(ability)
        let abilityPrefix = "ability/"
        guard path.hasPrefix(abilityPrefix) else {
            throw SDKError.validation("authority", "descriptor_ref must contain a canonical Ability URA")
        }
        let intrinsicName = String(path.dropFirst(abilityPrefix.count)).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !intrinsicName.isEmpty, !intrinsicName.contains("/") else {
            throw SDKError.validation("authority", "descriptor_ref must contain a canonical Ability URA")
        }
        return AbilityDescriptorProjection(abilityURA: ability, intrinsicName: intrinsicName)
    }

    private static func runtimeGovernanceReadAbility(_ value: String) -> String? {
        let clean = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return runtimeGovernanceReadAbilities.first { ability in
            clean == ability || clean.hasSuffix(".\(ability)")
        }
    }

    private static func publicAbilityName(calleeURA: String, intrinsicName: String) -> String {
        let clean = intrinsicName.trimmingCharacters(in: .whitespacesAndNewlines)
        let owner = abilityOwnerPrefix(calleeURA)
        if !owner.isEmpty, clean.hasPrefix("\(owner).") {
            return String(clean.dropFirst(owner.count + 1))
        }
        return ""
    }

    private static func abilityOwnerPrefix(_ calleeURA: String) -> String {
        let path = canonicalTopLevelPath(calleeURA)
        if path.hasPrefix("device/") {
            let deviceID = String(path.dropFirst("device/".count)).trimmingCharacters(in: .whitespacesAndNewlines)
            if !deviceID.isEmpty, !deviceID.contains("/") {
                return "device.\(deviceID)"
            }
        }
        if path == "authority" {
            return "authority"
        }
        return ""
    }

    private static func canonicalTopLevelPath(_ ura: String) -> String {
        let clean = ura.trimmingCharacters(in: .whitespacesAndNewlines)
        guard clean.hasPrefix(realmPrefix) else {
            return ""
        }
        let rest = String(clean.dropFirst(realmPrefix.count))
        guard let slash = rest.firstIndex(of: "/"), slash != rest.startIndex, slash != rest.index(before: rest.endIndex) else {
            return ""
        }
        return String(rest[rest.index(after: slash)...]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private struct AbilityDescriptorProjection: Sendable, Equatable {
        let abilityURA: String
        let intrinsicName: String
    }
}
