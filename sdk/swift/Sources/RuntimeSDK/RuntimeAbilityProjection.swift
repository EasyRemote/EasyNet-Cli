import Foundation

struct RuntimeAbilityProjection: Sendable, Equatable {
    private static let realmPrefix = "easynet:///r/"
    let abilityURA: String
    let publicName: String
    let intrinsicName: String
    let action: String

    private init(abilityURA: String, publicName: String, intrinsicName: String, action: String) {
        self.abilityURA = abilityURA
        self.publicName = publicName
        self.intrinsicName = intrinsicName
        self.action = action
    }

    init(tuple: InvocationTuple) throws {
        self = try Self.fromDescriptorRef(calleeURA: tuple.callee, descriptorRef: tuple.descriptorRef)
    }

    static func runtimeGovernanceReadAbility(calleeURA: String, descriptorRef: String) throws -> String? {
        let ability = try fromDescriptorRef(calleeURA: calleeURA, descriptorRef: descriptorRef)
        return runtimeGovernanceReadAbility(ability.publicName) ?? runtimeGovernanceReadAbility(ability.intrinsicName)
    }

    static func runtimeGovernanceDescriptorProvider(forAbility ability: String) -> String {
        RuntimeGovernanceRoutesGen.descriptorProvider(ability)
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
            intrinsicName: projection.intrinsicName,
            action: projection.action
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
        let action = if let bang {
            String(clean[clean.index(after: bang)...]).trimmingCharacters(in: .whitespacesAndNewlines)
        } else {
            "invoke"
        }
        return AbilityDescriptorProjection(
            abilityURA: ability,
            intrinsicName: intrinsicName,
            action: action.isEmpty ? "invoke" : action
        )
    }

    private static func runtimeGovernanceReadAbility(_ value: String) -> String? {
        RuntimeGovernanceRoutesGen.canonicalAbility(value)
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
        if path.hasPrefix("agent/device.") {
            let scopedAgentID = String(path.dropFirst("agent/device.".count)).trimmingCharacters(in: .whitespacesAndNewlines)
            if let separator = scopedAgentID.firstIndex(of: "."),
               separator != scopedAgentID.startIndex,
               separator != scopedAgentID.index(before: scopedAgentID.endIndex)
            {
                let deviceID = String(scopedAgentID[..<separator])
                let agentID = String(scopedAgentID[scopedAgentID.index(after: separator)...])
                return "system-agent.\(deviceID).\(agentID)"
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
        let action: String
    }
}
