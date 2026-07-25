import Foundation

struct RuntimeAbilityProjection: Sendable, Equatable {
    private static let abilityPathMarker = "/ability/"
    private static let realmPrefix = "easynet:///r/"

    let abilityURA: String
    let publicName: String

    init(tuple: InvocationTuple) throws {
        let abilityURA = try Self.descriptorAbilityURA(tuple.descriptorRef)
        self.abilityURA = abilityURA
        self.publicName = Self.publicAbilityName(calleeURA: tuple.callee, abilityURA: abilityURA)
    }

    private static func descriptorAbilityURA(_ descriptorRef: String) throws -> String {
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
        guard ability.hasPrefix(realmPrefix), ability.contains(abilityPathMarker) else {
            throw SDKError.validation("authority", "descriptor_ref must contain a canonical Ability URA")
        }
        return ability
    }

    private static func descriptorWireAbility(_ abilityURA: String) -> String {
        guard let range = abilityURA.range(of: abilityPathMarker) else {
            return abilityURA.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return String(abilityURA[range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func publicAbilityName(calleeURA: String, abilityURA: String) -> String {
        let clean = descriptorWireAbility(abilityURA)
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
}
