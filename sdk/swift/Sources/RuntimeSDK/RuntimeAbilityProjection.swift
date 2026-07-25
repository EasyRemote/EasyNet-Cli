import Foundation

struct RuntimeAbilityProjection: Sendable, Equatable {
    private static let abilityPathMarker = "/ability/"
    private static let realmPrefix = "easynet:///r/"

    let wire: String
    let abilityURA: String
    let publicName: String

    init(tuple: InvocationTuple) throws {
        let abilityURA = try Self.descriptorAbilityURA(tuple.descriptorRef)
        self.wire = Self.descriptorWireAbility(abilityURA)
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
        return clean
    }

    private static func abilityOwnerPrefix(_ calleeURA: String) -> String {
        let clean = calleeURA.trimmingCharacters(in: .whitespacesAndNewlines)
        let device = "/device/"
        if let range = clean.range(of: device) {
            let rest = String(clean[range.upperBound...])
            let id = rest
                .split(maxSplits: 1, whereSeparator: { $0 == "/" || $0 == "?" || $0 == "#" })
                .first
                .map(String.init) ?? ""
            return id.isEmpty ? "" : "device.\(id)"
        }
        guard clean.hasSuffix("/authority"), clean.hasPrefix(realmPrefix) else {
            return ""
        }
        let start = clean.index(clean.startIndex, offsetBy: realmPrefix.count)
        let end = clean.index(clean.endIndex, offsetBy: -"/authority".count)
        return "hub.\(String(clean[start..<end]))"
    }
}
