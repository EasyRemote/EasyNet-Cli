import Foundation

enum RuntimeSubjects {
    private static let runtimeStateReadSubjectPath = "runtime-state/read"

    static func runtimeStateReadSubjectURA(realm: String, userID: String) throws -> String {
        let cleanRealm = try RuntimePrincipals.requiredString(realm, "realm", stage: "runtime")
        let cleanUserID = try RuntimePrincipals.requiredPrincipalID(userID, "user_id", stage: "runtime")
        guard !cleanRealm.contains("/"), !cleanRealm.contains("?"), !cleanRealm.contains("#") else {
            throw invalidRuntime("runtime-state read subject realm is not canonical")
        }
        guard !cleanUserID.contains("/"), !cleanUserID.contains("?"), !cleanUserID.contains("#") else {
            throw invalidRuntime("runtime-state read subject user_id is not canonical")
        }
        let subject = "easynet:///r/\(cleanRealm)/resource/user.\(cleanUserID)/\(runtimeStateReadSubjectPath)"
        guard canonicalResourceSubject(subject) != nil else {
            throw invalidRuntime("runtime-state read subject_ura must be canonical")
        }
        return subject
    }

    static func descriptorBoundSubjectURA(_ subjectURA: String, abilityName: String) throws -> String {
        let subject = try RuntimePrincipals.requiredString(subjectURA, "subject_ura", stage: "runtime")
        let ability = try RuntimePrincipals.requiredString(abilityName, "ability name", stage: "runtime")
        guard let parsed = parsedSubject(subject) else {
            throw invalidRuntime("subject_ura is not a valid URA")
        }
        if parsed.path == "authority" {
            return "easynet:///r/\(parsed.realm)/resource/authority/invoke/\(ability)"
        }
        let userPrefix = "user/"
        if parsed.path.hasPrefix(userPrefix) {
            let userID = String(parsed.path.dropFirst(userPrefix.count)).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !userID.isEmpty,
                  !userID.contains("/"),
                  !userID.contains("?"),
                  !userID.contains("#")
            else {
                throw invalidRuntime("subject_ura user id is not canonical")
            }
            return "easynet:///r/\(parsed.realm)/resource/user.\(userID)/invoke/\(ability)"
        }
        if parsed.path.hasPrefix("agent/") ||
            parsed.path.hasPrefix("ability/") ||
            parsed.path.hasPrefix("device/") ||
            parsed.path.hasPrefix("resource/")
        {
            return subject
        }
        throw invalidRuntime("subject_ura kind is not descriptor-bound")
    }

    static func canonicalResourceSubject(_ subjectURA: String) -> ResourceSubject? {
        guard !RuntimePrincipals.containsAllZeroPrincipal(subjectURA) else {
            return nil
        }
        let raw = subjectURA.trimmingCharacters(in: .whitespacesAndNewlines)
        let realmPrefix = "easynet:///r/"
        guard raw.hasPrefix(realmPrefix) else {
            return nil
        }
        let rest = String(raw.dropFirst(realmPrefix.count))
        guard let slash = rest.firstIndex(of: "/"), slash != rest.startIndex else {
            return nil
        }
        let path = String(rest[rest.index(after: slash)...])
        let resourcePrefix = "resource/"
        guard path.hasPrefix(resourcePrefix) else {
            return nil
        }
        let resource = String(path.dropFirst(resourcePrefix.count))
        guard let pathSlash = resource.firstIndex(of: "/"),
              pathSlash != resource.startIndex,
              pathSlash != resource.index(before: resource.endIndex)
        else {
            return nil
        }
        let ownerID = String(resource[..<pathSlash]).trimmingCharacters(in: .whitespacesAndNewlines)
        let resourcePath = String(resource[resource.index(after: pathSlash)...]).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !ownerID.isEmpty,
              !ownerID.contains("/"),
              !resourcePath.isEmpty,
              !resourcePath.hasPrefix("/"),
              !resourcePath.contains("//")
        else {
            return nil
        }
        return ResourceSubject(ownerID: ownerID, path: resourcePath)
    }

    static func isRuntimeGovernanceReadSubject(_ subjectURA: String, calleeURA: String) -> Bool {
        let subject = subjectURA.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !subject.isEmpty, !RuntimePrincipals.containsAllZeroPrincipal(subject) else {
            return false
        }
        if let resource = canonicalResourceSubject(subject) {
            let userPrefix = "user."
            guard resource.ownerID.hasPrefix(userPrefix) else {
                return false
            }
            let userID = String(resource.ownerID.dropFirst(userPrefix.count)).trimmingCharacters(in: .whitespacesAndNewlines)
            return !userID.isEmpty &&
                !userID.contains(".") &&
                !RuntimePrincipals.containsAllZeroPrincipal(userID) &&
                resource.path == runtimeStateReadSubjectPath
        }
        guard let parsedSubject = runtimeOwnerSubject(subject),
              let parsedCallee = runtimeOwnerSubject(calleeURA)
        else {
            return false
        }
        return parsedSubject.kind == parsedCallee.kind &&
            parsedSubject.realm == parsedCallee.realm &&
            subject == calleeURA.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func runtimeOwnerSubject(_ ura: String) -> RuntimeOwnerSubject? {
        guard let parsed = parsedSubject(ura) else { return nil }
        let realm = parsed.realm
        let path = parsed.path
        guard !realm.isEmpty, !realm.contains("/") else {
            return nil
        }
        if path == "authority" {
            return RuntimeOwnerSubject(kind: "authority", realm: realm)
        }
        let devicePrefix = "device/"
        if path.hasPrefix(devicePrefix) {
            let deviceID = String(path.dropFirst(devicePrefix.count)).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !deviceID.isEmpty, !deviceID.contains("/") else {
                return nil
            }
            return RuntimeOwnerSubject(kind: "device", realm: realm)
        }
        return nil
    }

    private static func parsedSubject(_ ura: String) -> ParsedSubject? {
        let raw = ura.trimmingCharacters(in: .whitespacesAndNewlines)
        let realmPrefix = "easynet:///r/"
        guard raw.hasPrefix(realmPrefix), !RuntimePrincipals.containsAllZeroPrincipal(raw) else {
            return nil
        }
        let rest = String(raw.dropFirst(realmPrefix.count))
        guard let slash = rest.firstIndex(of: "/"),
              slash != rest.startIndex,
              slash != rest.index(before: rest.endIndex)
        else {
            return nil
        }
        let realm = String(rest[..<slash]).trimmingCharacters(in: .whitespacesAndNewlines)
        let path = String(rest[rest.index(after: slash)...]).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !realm.isEmpty, !realm.contains("/"), !path.isEmpty else {
            return nil
        }
        return ParsedSubject(realm: realm, path: path)
    }

    static func canonicalSessionAuthorityID(_ sessionID: String) -> Bool {
        let cleaned = sessionID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleaned.isEmpty else {
            return false
        }
        return cleaned.unicodeScalars.allSatisfy { scalar in
            let value = scalar.value
            return (value >= 0x61 && value <= 0x7A) ||
                (value >= 0x41 && value <= 0x5A) ||
                (value >= 0x30 && value <= 0x39) ||
                value == 0x2D ||
                value == 0x2E
        }
    }

    struct ResourceSubject {
        let ownerID: String
        let path: String
    }

    private struct RuntimeOwnerSubject {
        let kind: String
        let realm: String
    }

    private struct ParsedSubject {
        let realm: String
        let path: String
    }

    private static func invalidRuntime(_ message: String) -> SDKError {
        SDKError.validation("runtime", message)
    }
}
