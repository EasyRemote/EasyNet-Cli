import Foundation

enum RuntimePrincipals {
    private static let allZeroPrincipalID = "00000000-0000-0000-0000-000000000000"

    static func requiredString(_ value: String, _ field: String, stage: String) throws -> String {
        guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              value == value.trimmingCharacters(in: .whitespacesAndNewlines)
        else {
            throw SDKError.validation(stage, "\(field) is required")
        }
        return value
    }

    static func requiredPrincipalID(_ value: String, _ field: String, stage: String) throws -> String {
        let cleaned = try requiredString(value, field, stage: stage)
        if containsAllZeroPrincipal(cleaned) {
            throw SDKError.validation(stage, "\(field) must not be all-zero")
        }
        return cleaned
    }

    static func containsAllZeroPrincipal(_ value: String) -> Bool {
        value.trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .contains(allZeroPrincipalID)
    }
}
