import Foundation

public let compatibilityProfile = "compatibility"

public struct CompatibilityCarrierBase: Sendable, Equatable {
    public let callerURA: String
    public let calleeURA: String
    public let subjectURA: String
    public let descriptorVersion: String
    public let nonceBase64: String
    public let causalContext: [String: JSONValue]
    public let authToken: String?
    public let metadata: [String: JSONValue]

    public init(
        callerURA: String,
        calleeURA: String,
        subjectURA: String,
        descriptorVersion: String,
        nonceBase64: String,
        causalContext: [String: JSONValue],
        authToken: String? = nil,
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.callerURA = try requiredCompatibilityURA(callerURA, "caller_ura")
        self.calleeURA = try requiredCompatibilityURA(calleeURA, "callee_ura")
        self.subjectURA = try requiredCompatibilityURA(subjectURA, "subject_ura")
        self.descriptorVersion = try requiredCompatibilityString(descriptorVersion, "descriptor_version")
        self.nonceBase64 = try requiredCompatibilityString(nonceBase64, "nonce_base64")
        guard !causalContext.isEmpty else {
            throw invalidCompatibility("causal_context is required")
        }
        self.causalContext = causalContext
        self.authToken = try optionalCompatibilityString(authToken.map(JSONValue.string), "auth_token")
        self.metadata = metadata
    }

    func jsonObject() -> [String: JSONValue] {
        var object: [String: JSONValue] = [
            "caller_ura": .string(callerURA),
            "callee_ura": .string(calleeURA),
            "subject_ura": .string(subjectURA),
            "descriptor_version": .string(descriptorVersion),
            "nonce_base64": .string(nonceBase64),
            "causal_context": .object(causalContext),
        ]
        if let authToken { object["auth_token"] = .string(authToken) }
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return object
    }

    static func fromObject(_ object: [String: JSONValue]) throws -> CompatibilityCarrierBase {
        try CompatibilityCarrierBase(
            callerURA: requiredCompatibilityString(object, "caller_ura"),
            calleeURA: requiredCompatibilityString(object, "callee_ura"),
            subjectURA: requiredCompatibilityString(object, "subject_ura"),
            descriptorVersion: requiredCompatibilityString(object, "descriptor_version"),
            nonceBase64: requiredCompatibilityString(object, "nonce_base64"),
            causalContext: requiredCompatibilityObject(object, "causal_context"),
            authToken: optionalCompatibilityString(object["auth_token"], "auth_token"),
            metadata: object["metadata"] == nil ? [:] : requiredCompatibilityObject(object, "metadata")
        )
    }
}

public struct CompatibilityListModelsRequest: Sendable, Equatable {
    public let base: CompatibilityCarrierBase

    public init(base: CompatibilityCarrierBase) {
        self.base = base
    }

    func jsonData() throws -> Data {
        try encodeJSONObject(base.jsonObject())
    }

    public static func fromJSON(_ raw: Data) throws -> CompatibilityListModelsRequest {
        try CompatibilityListModelsRequest(
            base: CompatibilityCarrierBase.fromObject(
                decodeCompatibilityObject(raw, label: "compatibility list models request JSON")
            )
        )
    }
}

public struct CompatibilityChatCompletionRequest: Sendable, Equatable {
    public let base: CompatibilityCarrierBase
    public let request: [String: JSONValue]

    public init(base: CompatibilityCarrierBase, request: [String: JSONValue]) throws {
        self.base = base
        self.request = try validateCompatibilityChatRequest(request, stream: false)
    }

    func jsonData() throws -> Data {
        var object = base.jsonObject()
        object["request"] = .object(request)
        return try encodeJSONObject(object)
    }

    public static func fromJSON(_ raw: Data) throws -> CompatibilityChatCompletionRequest {
        let object = try decodeCompatibilityObject(raw, label: "compatibility chat completion request JSON")
        return try CompatibilityChatCompletionRequest(
            base: CompatibilityCarrierBase.fromObject(object),
            request: requiredCompatibilityObject(object, "request")
        )
    }
}

public struct CompatibilityStreamChatCompletionRequest: Sendable, Equatable {
    public let base: CompatibilityCarrierBase
    public let request: [String: JSONValue]

    public init(base: CompatibilityCarrierBase, request: [String: JSONValue]) throws {
        self.base = base
        self.request = try validateCompatibilityChatRequest(request, stream: true)
    }

    func jsonData() throws -> Data {
        var object = base.jsonObject()
        object["request"] = .object(request)
        return try encodeJSONObject(object)
    }

    public static func fromJSON(_ raw: Data) throws -> CompatibilityStreamChatCompletionRequest {
        let object = try decodeCompatibilityObject(raw, label: "compatibility stream chat completion request JSON")
        return try CompatibilityStreamChatCompletionRequest(
            base: CompatibilityCarrierBase.fromObject(object),
            request: requiredCompatibilityObject(object, "request")
        )
    }
}

public struct CompatibilityFileUploadRequest: Sendable, Equatable {
    public let base: CompatibilityCarrierBase?
    public let id: String
    public let fileRef: String
    public let ownerURA: String
    public let filename: String
    public let purpose: String
    public let contentType: String
    public let contentHash: String
    public let sizeBytes: Int64
    public let createdAt: Int64
    public let metadata: [String: JSONValue]

    public init(
        base: CompatibilityCarrierBase? = nil,
        id: String,
        fileRef: String,
        ownerURA: String,
        filename: String,
        purpose: String,
        contentType: String,
        contentHash: String,
        sizeBytes: Int64,
        createdAt: Int64,
        metadata: [String: JSONValue] = [:]
    ) throws {
        self.base = base
        self.id = try requiredCompatibilityString(id, "id")
        self.fileRef = try requiredCompatibilityURA(fileRef, "file_ref")
        self.ownerURA = try requiredCompatibilityURA(ownerURA, "owner_ura")
        self.filename = try requiredCompatibilityString(filename, "filename")
        self.purpose = try requiredCompatibilityString(purpose, "purpose")
        self.contentType = try requiredCompatibilityString(contentType, "content_type")
        self.contentHash = try requiredCompatibilityString(contentHash, "content_hash")
        try validateCompatibilityHash(self.contentHash, "content_hash")
        if sizeBytes < 0 { throw invalidCompatibility("size_bytes must be a non-negative integer") }
        if createdAt < 0 { throw invalidCompatibility("created_at must be a non-negative integer") }
        self.sizeBytes = sizeBytes
        self.createdAt = createdAt
        self.metadata = metadata
    }

    func jsonData() throws -> Data {
        var object = base?.jsonObject() ?? [:]
        object.merge([
            "id": .string(id),
            "file_ref": .string(fileRef),
            "owner_ura": .string(ownerURA),
            "filename": .string(filename),
            "purpose": .string(purpose),
            "content_type": .string(contentType),
            "content_hash": .string(contentHash),
            "size_bytes": .number(Double(sizeBytes)),
            "created_at": .number(Double(createdAt)),
        ]) { _, new in new }
        if !metadata.isEmpty { object["metadata"] = .object(metadata) }
        return try encodeJSONObject(object)
    }

    public static func fromJSON(_ raw: Data) throws -> CompatibilityFileUploadRequest {
        let object = try decodeCompatibilityObject(raw, label: "compatibility file upload request JSON")
        return try CompatibilityFileUploadRequest(
            base: object["caller_ura"] == nil ? nil : CompatibilityCarrierBase.fromObject(object),
            id: requiredCompatibilityString(object, "id"),
            fileRef: requiredCompatibilityString(object, "file_ref"),
            ownerURA: requiredCompatibilityString(object, "owner_ura"),
            filename: requiredCompatibilityString(object, "filename"),
            purpose: requiredCompatibilityString(object, "purpose"),
            contentType: requiredCompatibilityString(object, "content_type"),
            contentHash: requiredCompatibilityString(object, "content_hash"),
            sizeBytes: requiredCompatibilityInteger(object["size_bytes"], "size_bytes"),
            createdAt: requiredCompatibilityInteger(object["created_at"], "created_at"),
            metadata: object["metadata"] == nil ? [:] : requiredCompatibilityObject(object, "metadata")
        )
    }
}

public struct CompatibilityFileRequest: Sendable, Equatable {
    public let base: CompatibilityCarrierBase?
    public let id: String
    public let fileRef: String
    public let ownerURA: String
    public let filename: String
    public let purpose: String
    public let contentType: String
    public let contentHash: String
    public let sizeBytes: Int64
    public let createdAt: Int64

    public init(
        base: CompatibilityCarrierBase? = nil,
        id: String,
        fileRef: String,
        ownerURA: String,
        filename: String,
        purpose: String,
        contentType: String,
        contentHash: String,
        sizeBytes: Int64,
        createdAt: Int64
    ) throws {
        self.base = base
        self.id = try requiredCompatibilityString(id, "id")
        self.fileRef = try requiredCompatibilityURA(fileRef, "file_ref")
        self.ownerURA = try requiredCompatibilityURA(ownerURA, "owner_ura")
        self.filename = try requiredCompatibilityString(filename, "filename")
        self.purpose = try requiredCompatibilityString(purpose, "purpose")
        self.contentType = try requiredCompatibilityString(contentType, "content_type")
        self.contentHash = try requiredCompatibilityString(contentHash, "content_hash")
        try validateCompatibilityHash(self.contentHash, "content_hash")
        if sizeBytes < 0 { throw invalidCompatibility("size_bytes must be a non-negative integer") }
        if createdAt < 0 { throw invalidCompatibility("created_at must be a non-negative integer") }
        self.sizeBytes = sizeBytes
        self.createdAt = createdAt
    }

    func jsonData() throws -> Data {
        var object = base?.jsonObject() ?? [:]
        object.merge([
            "id": .string(id),
            "file_ref": .string(fileRef),
            "owner_ura": .string(ownerURA),
            "filename": .string(filename),
            "purpose": .string(purpose),
            "content_type": .string(contentType),
            "content_hash": .string(contentHash),
            "size_bytes": .number(Double(sizeBytes)),
            "created_at": .number(Double(createdAt)),
        ]) { _, new in new }
        return try encodeJSONObject(object)
    }

    public static func fromJSON(_ raw: Data) throws -> CompatibilityFileRequest {
        let object = try decodeCompatibilityObject(raw, label: "compatibility file request JSON")
        return try CompatibilityFileRequest(
            base: object["caller_ura"] == nil ? nil : CompatibilityCarrierBase.fromObject(object),
            id: requiredCompatibilityString(object, "id"),
            fileRef: requiredCompatibilityString(object, "file_ref"),
            ownerURA: requiredCompatibilityString(object, "owner_ura"),
            filename: requiredCompatibilityString(object, "filename"),
            purpose: requiredCompatibilityString(object, "purpose"),
            contentType: requiredCompatibilityString(object, "content_type"),
            contentHash: requiredCompatibilityString(object, "content_hash"),
            sizeBytes: requiredCompatibilityInteger(object["size_bytes"], "size_bytes"),
            createdAt: requiredCompatibilityInteger(object["created_at"], "created_at")
        )
    }
}

public struct CompatibilityFileDeleteRequest: Sendable, Equatable {
    public let base: CompatibilityCarrierBase?
    public let id: String
    public let deleted: Bool

    public init(base: CompatibilityCarrierBase? = nil, id: String, deleted: Bool) throws {
        self.base = base
        self.id = try requiredCompatibilityString(id, "id")
        guard deleted else { throw invalidCompatibility("deleted must be true") }
        self.deleted = true
    }

    func jsonData() throws -> Data {
        var object = base?.jsonObject() ?? [:]
        object["id"] = .string(id)
        object["deleted"] = .bool(true)
        return try encodeJSONObject(object)
    }

    public static func fromJSON(_ raw: Data) throws -> CompatibilityFileDeleteRequest {
        let object = try decodeCompatibilityObject(raw, label: "compatibility file delete request JSON")
        return try CompatibilityFileDeleteRequest(
            base: object["caller_ura"] == nil ? nil : CompatibilityCarrierBase.fromObject(object),
            id: requiredCompatibilityString(object, "id"),
            deleted: requiredCompatibilityBool(object, "deleted")
        )
    }
}

public struct CompatibilityModel: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let id: String
    public let object: String
    public let created: Int64
    public let ownedBy: String
    public let abilityRef: String
    public let metadata: [String: JSONValue]

    static func fromObject(_ object: [String: JSONValue]) throws -> CompatibilityModel {
        let model = try CompatibilityModel(
            profile: requiredCompatibilityString(object, "profile"),
            kind: requiredCompatibilityString(object, "kind"),
            id: requiredCompatibilityString(object, "id"),
            object: requiredCompatibilityString(object, "object"),
            created: requiredCompatibilityInteger(object["created"], "created"),
            ownedBy: requiredCompatibilityString(object, "owned_by"),
            abilityRef: requiredCompatibilityString(object, "ability_ref"),
            metadata: requiredCompatibilityObject(object, "metadata")
        )
        guard model.profile == compatibilityProfile, model.kind == "model", model.object == "model" else {
            throw invalidCompatibility("invalid model projection")
        }
        _ = try requiredCompatibilityAbilityURA(model.id, "id")
        _ = try requiredCompatibilityAbilityURA(model.abilityRef, "ability_ref")
        return model
    }

    func jsonObject() -> [String: JSONValue] {
        [
            "profile": .string(profile),
            "kind": .string(kind),
            "id": .string(id),
            "object": .string(object),
            "created": .number(Double(created)),
            "owned_by": .string(ownedBy),
            "ability_ref": .string(abilityRef),
            "metadata": .object(metadata),
        ]
    }
}

public struct CompatibilityModelPage: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let object: String
    public let data: [CompatibilityModel]
    public let nextCursor: String?
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> CompatibilityModelPage {
        let object = try decodeCompatibilityObject(raw, label: "compatibility model page JSON")
        let page = try CompatibilityModelPage(
            profile: requiredCompatibilityString(object, "profile"),
            kind: requiredCompatibilityString(object, "kind"),
            object: requiredCompatibilityString(object, "object"),
            data: requiredCompatibilityArray(object, "data").map {
                guard case let .object(item) = $0 else { throw invalidCompatibility("model entry must be an object") }
                return try CompatibilityModel.fromObject(item)
            },
            nextCursor: optionalCompatibilityString(object["next_cursor"], "next_cursor"),
            metadata: requiredCompatibilityObject(object, "metadata")
        )
        guard page.profile == compatibilityProfile, page.kind == "model_page", page.object == "list" else {
            throw invalidCompatibility("invalid model_page projection")
        }
        return page
    }

    func jsonData() throws -> Data {
        try encodeJSONObject([
            "profile": .string(profile),
            "kind": .string(kind),
            "object": .string(object),
            "data": .array(data.map { .object($0.jsonObject()) }),
            "next_cursor": nextCursor.map(JSONValue.string) ?? .null,
            "metadata": .object(metadata),
        ])
    }
}

public struct CompatibilityChatCompletion: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let id: String
    public let object: String
    public let created: Int64
    public let model: String
    public let choices: [JSONValue]
    public let usage: [String: JSONValue]
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> CompatibilityChatCompletion {
        let object = try decodeCompatibilityObject(raw, label: "compatibility chat completion JSON")
        let completion = try CompatibilityChatCompletion(
            profile: requiredCompatibilityString(object, "profile"),
            kind: requiredCompatibilityString(object, "kind"),
            id: requiredCompatibilityString(object, "id"),
            object: requiredCompatibilityString(object, "object"),
            created: requiredCompatibilityInteger(object["created"], "created"),
            model: requiredCompatibilityString(object, "model"),
            choices: requiredCompatibilityArray(object, "choices"),
            usage: requiredCompatibilityObject(object, "usage"),
            metadata: requiredCompatibilityObject(object, "metadata")
        )
        guard completion.profile == compatibilityProfile,
              completion.kind == "chat_completion",
              completion.object == "chat.completion"
        else {
            throw invalidCompatibility("invalid chat_completion projection")
        }
        _ = try requiredCompatibilityAbilityURA(completion.model, "model")
        return completion
    }

    func jsonData() throws -> Data {
        try encodeJSONObject([
            "profile": .string(profile),
            "kind": .string(kind),
            "id": .string(id),
            "object": .string(object),
            "created": .number(Double(created)),
            "model": .string(model),
            "choices": .array(choices),
            "usage": .object(usage),
            "metadata": .object(metadata),
        ])
    }
}

public struct CompatibilityChatCompletionChunk: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let id: String
    public let object: String
    public let created: Int64
    public let model: String
    public let choices: [JSONValue]
    public let usage: [String: JSONValue]?
    public let metadata: [String: JSONValue]

    static func fromObject(_ object: [String: JSONValue]) throws -> CompatibilityChatCompletionChunk {
        let chunk = try CompatibilityChatCompletionChunk(
            profile: requiredCompatibilityString(object, "profile"),
            kind: requiredCompatibilityString(object, "kind"),
            id: requiredCompatibilityString(object, "id"),
            object: requiredCompatibilityString(object, "object"),
            created: requiredCompatibilityInteger(object["created"], "created"),
            model: requiredCompatibilityString(object, "model"),
            choices: requiredCompatibilityArray(object, "choices"),
            usage: optionalCompatibilityObject(object["usage"], "usage"),
            metadata: requiredCompatibilityObject(object, "metadata")
        )
        guard chunk.profile == compatibilityProfile,
              chunk.kind == "chat_completion_chunk",
              chunk.object == "chat.completion.chunk"
        else {
            throw invalidCompatibility("invalid chat_completion_chunk projection")
        }
        _ = try requiredCompatibilityAbilityURA(chunk.model, "model")
        return chunk
    }

    func jsonObject() -> [String: JSONValue] {
        [
            "profile": .string(profile),
            "kind": .string(kind),
            "id": .string(id),
            "object": .string(object),
            "created": .number(Double(created)),
            "model": .string(model),
            "choices": .array(choices),
            "usage": usage.map(JSONValue.object) ?? .null,
            "metadata": .object(metadata),
        ]
    }
}

public struct CompatibilityChatCompletionStream: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let stream: Bool
    public let items: [CompatibilityChatCompletionChunk]
    public let doneSentinel: String
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> CompatibilityChatCompletionStream {
        let object = try decodeCompatibilityObject(raw, label: "compatibility chat stream JSON")
        let stream = try CompatibilityChatCompletionStream(
            profile: requiredCompatibilityString(object, "profile"),
            kind: requiredCompatibilityString(object, "kind"),
            stream: requiredCompatibilityBool(object, "stream"),
            items: requiredCompatibilityArray(object, "items").map {
                guard case let .object(item) = $0 else { throw invalidCompatibility("stream item must be an object") }
                return try CompatibilityChatCompletionChunk.fromObject(item)
            },
            doneSentinel: requiredCompatibilityString(object, "done_sentinel"),
            metadata: requiredCompatibilityObject(object, "metadata")
        )
        guard stream.profile == compatibilityProfile,
              stream.kind == "chat_completion_stream",
              stream.stream,
              stream.doneSentinel == "[DONE]"
        else {
            throw invalidCompatibility("invalid chat_completion_stream projection")
        }
        return stream
    }

    func jsonData() throws -> Data {
        try encodeJSONObject([
            "profile": .string(profile),
            "kind": .string(kind),
            "stream": .bool(stream),
            "items": .array(items.map { .object($0.jsonObject()) }),
            "done_sentinel": .string(doneSentinel),
            "metadata": .object(metadata),
        ])
    }
}

public struct CompatibilityFile: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let id: String
    public let object: String
    public let bytes: Int64
    public let createdAt: Int64
    public let filename: String
    public let purpose: String
    public let status: String
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> CompatibilityFile {
        let object = try decodeCompatibilityObject(raw, label: "compatibility file JSON")
        let file = try CompatibilityFile(
            profile: requiredCompatibilityString(object, "profile"),
            kind: requiredCompatibilityString(object, "kind"),
            id: requiredCompatibilityString(object, "id"),
            object: requiredCompatibilityString(object, "object"),
            bytes: requiredCompatibilityInteger(object["bytes"], "bytes"),
            createdAt: requiredCompatibilityInteger(object["created_at"], "created_at"),
            filename: requiredCompatibilityString(object, "filename"),
            purpose: requiredCompatibilityString(object, "purpose"),
            status: requiredCompatibilityString(object, "status"),
            metadata: requiredCompatibilityObject(object, "metadata")
        )
        guard file.profile == compatibilityProfile, file.kind == "file", file.object == "file" else {
            throw invalidCompatibility("invalid file projection")
        }
        return file
    }

    func jsonData() throws -> Data {
        try encodeJSONObject([
            "profile": .string(profile),
            "kind": .string(kind),
            "id": .string(id),
            "object": .string(object),
            "bytes": .number(Double(bytes)),
            "created_at": .number(Double(createdAt)),
            "filename": .string(filename),
            "purpose": .string(purpose),
            "status": .string(status),
            "metadata": .object(metadata),
        ])
    }
}

public struct CompatibilityFileDeleteResult: Sendable, Equatable {
    public let profile: String
    public let kind: String
    public let id: String
    public let object: String
    public let deleted: Bool
    public let metadata: [String: JSONValue]

    public static func fromJSON(_ raw: Data) throws -> CompatibilityFileDeleteResult {
        let object = try decodeCompatibilityObject(raw, label: "compatibility file delete JSON")
        let result = try CompatibilityFileDeleteResult(
            profile: requiredCompatibilityString(object, "profile"),
            kind: requiredCompatibilityString(object, "kind"),
            id: requiredCompatibilityString(object, "id"),
            object: requiredCompatibilityString(object, "object"),
            deleted: requiredCompatibilityBool(object, "deleted"),
            metadata: requiredCompatibilityObject(object, "metadata")
        )
        guard result.profile == compatibilityProfile,
              result.kind == "file_delete_result",
              result.object == "file",
              result.deleted
        else {
            throw invalidCompatibility("invalid file_delete_result projection")
        }
        return result
    }

    func jsonData() throws -> Data {
        try encodeJSONObject([
            "profile": .string(profile),
            "kind": .string(kind),
            "id": .string(id),
            "object": .string(object),
            "deleted": .bool(true),
            "metadata": .object(metadata),
        ])
    }
}

public protocol CompatibilityTransport: AnyObject, Sendable {
    func buildListModelsInvocation(_ requestJSON: Data) async throws -> Data
    func buildChatCompletionInvocation(_ requestJSON: Data) async throws -> Data
    func buildStreamChatCompletionInvocation(_ requestJSON: Data) async throws -> Data
    func listModels(_ requestJSON: Data) async throws -> Data
    func chatCompletions(_ requestJSON: Data) async throws -> Data
    func streamChatCompletions(_ requestJSON: Data) async throws -> Data
    func uploadFile(_ requestJSON: Data) async throws -> Data
    func getFile(_ requestJSON: Data) async throws -> Data
    func deleteFile(_ requestJSON: Data) async throws -> Data
    func projectModelPage(_ valueJSON: Data) async throws -> Data
    func projectChatCompletion(_ valueJSON: Data) async throws -> Data
    func projectChatStream(_ valueJSON: Data) async throws -> Data
    func projectFileUpload(_ valueJSON: Data) async throws -> Data
    func projectFile(_ valueJSON: Data) async throws -> Data
    func projectFileDeleteResult(_ valueJSON: Data) async throws -> Data
    func close() async throws
}

public extension CompatibilityTransport {
    func buildListModelsInvocation(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility list-models invocation transport is not available") }
    func buildChatCompletionInvocation(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility chat invocation transport is not available") }
    func buildStreamChatCompletionInvocation(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility stream invocation transport is not available") }
    func listModels(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility list-models transport is not available") }
    func chatCompletions(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility chat transport is not available") }
    func streamChatCompletions(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility stream transport is not available") }
    func uploadFile(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility file upload transport is not available") }
    func getFile(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility file get transport is not available") }
    func deleteFile(_ requestJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility file delete transport is not available") }
    func projectModelPage(_ valueJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility model projection transport is not available") }
    func projectChatCompletion(_ valueJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility chat projection transport is not available") }
    func projectChatStream(_ valueJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility stream projection transport is not available") }
    func projectFileUpload(_ valueJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility file upload projection transport is not available") }
    func projectFile(_ valueJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility file projection transport is not available") }
    func projectFileDeleteResult(_ valueJSON: Data) async throws -> Data { throw compatibilityUnsupported("compatibility file delete projection transport is not available") }
    func close() async throws {}
}

public final class CompatibilityClient: @unchecked Sendable {
    private let transport: CompatibilityTransport
    private var closed = false

    public init(transport: CompatibilityTransport) {
        self.transport = transport
    }

    public func buildListModelsInvocation(_ request: CompatibilityListModelsRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildListModelsInvocation(request.jsonData()) }
    }

    public func buildChatCompletionInvocation(_ request: CompatibilityChatCompletionRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildChatCompletionInvocation(request.jsonData()) }
    }

    public func buildStreamChatCompletionInvocation(_ request: CompatibilityStreamChatCompletionRequest) async throws -> [String: JSONValue] {
        try await carrier { try await transport.buildStreamChatCompletionInvocation(request.jsonData()) }
    }

    public func listModels(_ request: CompatibilityListModelsRequest) async throws -> CompatibilityModelPage {
        try await CompatibilityModelPage.fromJSON(raw { try await transport.listModels(request.jsonData()) })
    }

    public func chatCompletions(_ request: CompatibilityChatCompletionRequest) async throws -> CompatibilityChatCompletion {
        try await CompatibilityChatCompletion.fromJSON(raw { try await transport.chatCompletions(request.jsonData()) })
    }

    public func streamChatCompletions(_ request: CompatibilityStreamChatCompletionRequest) async throws -> CompatibilityChatCompletionStream {
        try await CompatibilityChatCompletionStream.fromJSON(raw { try await transport.streamChatCompletions(request.jsonData()) })
    }

    public func uploadFile(_ request: CompatibilityFileUploadRequest) async throws -> CompatibilityFile {
        try await CompatibilityFile.fromJSON(raw { try await transport.uploadFile(request.jsonData()) })
    }

    public func getFile(_ request: CompatibilityFileRequest) async throws -> CompatibilityFile {
        try await CompatibilityFile.fromJSON(raw { try await transport.getFile(request.jsonData()) })
    }

    public func deleteFile(_ request: CompatibilityFileDeleteRequest) async throws -> CompatibilityFileDeleteResult {
        try await CompatibilityFileDeleteResult.fromJSON(raw { try await transport.deleteFile(request.jsonData()) })
    }

    public func projectModelPage(_ rawValue: Data) async throws -> CompatibilityModelPage {
        try await CompatibilityModelPage.fromJSON(raw { try await transport.projectModelPage(rawValue) })
    }

    public func projectModelPage(_ value: CompatibilityModelPage) async throws -> CompatibilityModelPage {
        try await projectModelPage(value.jsonData())
    }

    public func projectChatCompletion(_ rawValue: Data) async throws -> CompatibilityChatCompletion {
        try await CompatibilityChatCompletion.fromJSON(raw { try await transport.projectChatCompletion(rawValue) })
    }

    public func projectChatCompletion(_ value: CompatibilityChatCompletion) async throws -> CompatibilityChatCompletion {
        try await projectChatCompletion(value.jsonData())
    }

    public func projectChatStream(_ rawValue: Data) async throws -> CompatibilityChatCompletionStream {
        try await CompatibilityChatCompletionStream.fromJSON(raw { try await transport.projectChatStream(rawValue) })
    }

    public func projectChatStream(_ value: CompatibilityChatCompletionStream) async throws -> CompatibilityChatCompletionStream {
        try await projectChatStream(value.jsonData())
    }

    public func projectFileUpload(_ value: CompatibilityFileUploadRequest) async throws -> CompatibilityFile {
        try await CompatibilityFile.fromJSON(raw { try await transport.projectFileUpload(value.jsonData()) })
    }

    public func projectFile(_ rawValue: Data) async throws -> CompatibilityFile {
        try await CompatibilityFile.fromJSON(raw { try await transport.projectFile(rawValue) })
    }

    public func projectFile(_ value: CompatibilityFileRequest) async throws -> CompatibilityFile {
        try await projectFile(value.jsonData())
    }

    public func projectFile(_ value: CompatibilityFile) async throws -> CompatibilityFile {
        try await projectFile(value.jsonData())
    }

    public func projectFileDeleteResult(_ rawValue: Data) async throws -> CompatibilityFileDeleteResult {
        try await CompatibilityFileDeleteResult.fromJSON(raw { try await transport.projectFileDeleteResult(rawValue) })
    }

    public func projectFileDeleteResult(_ value: CompatibilityFileDeleteRequest) async throws -> CompatibilityFileDeleteResult {
        try await projectFileDeleteResult(value.jsonData())
    }

    public func projectFileDeleteResult(_ value: CompatibilityFileDeleteResult) async throws -> CompatibilityFileDeleteResult {
        try await projectFileDeleteResult(value.jsonData())
    }

    public func close() async throws {
        guard !closed else { return }
        closed = true
        try await transport.close()
    }

    private func carrier(_ call: () async throws -> Data) async throws -> [String: JSONValue] {
        try decodeCompatibilityObject(try await raw(call), label: "compatibility invocation JSON")
    }

    private func raw(_ call: () async throws -> Data) async throws -> Data {
        try requireOpen()
        do {
            return try await call()
        } catch let error as SDKError {
            throw error
        } catch {
            throw SDKError(
                code: .transport,
                stage: "transport",
                retryHint: .safe,
                retryable: true,
                message: "compatibility transport failed",
                details: ["profile": compatibilityProfile]
            )
        }
    }

    private func requireOpen() throws {
        if closed { throw SDKError.closed(compatibilityProfile) }
    }
}

private func validateCompatibilityChatRequest(_ request: [String: JSONValue], stream: Bool) throws -> [String: JSONValue] {
    var object = request
    object["model"] = .string(try requiredCompatibilityAbilityURA(requiredCompatibilityString(object, "model"), "model"))
    guard case let .array(messages) = object["messages"], !messages.isEmpty else {
        throw invalidCompatibility("messages must be a non-empty array")
    }
    for message in messages {
        guard case .object = message else { throw invalidCompatibility("message must be an object") }
    }
    if !stream, case .bool(true) = object["stream"] {
        throw invalidCompatibility("unary chat completion request must not set stream=true")
    }
    if stream {
        object["stream"] = .bool(true)
    }
    return object
}

private func decodeCompatibilityObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw invalidCompatibility("\(label) must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError(code: .invalidArgument, stage: "decode", message: "decode \(label): \(error)", details: ["profile": compatibilityProfile])
    }
}

private func requiredCompatibilityString(_ value: String, _ field: String) throws -> String {
    guard !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines) else {
        throw invalidCompatibility("\(field) is required")
    }
    return value
}

private func requiredCompatibilityString(_ object: [String: JSONValue], _ name: String) throws -> String {
    if case let .string(value) = object[name], !value.isEmpty, value == value.trimmingCharacters(in: .whitespacesAndNewlines) {
        return value
    }
    throw invalidCompatibility("\(name) is required")
}

private func optionalCompatibilityString(_ value: JSONValue?, _ name: String) throws -> String? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .string(string):
        guard string == string.trimmingCharacters(in: .whitespacesAndNewlines) else {
            throw invalidCompatibility("\(name) must be a string or null")
        }
        return string
    default:
        throw invalidCompatibility("\(name) must be a string or null")
    }
}

private func requiredCompatibilityURA(_ value: String, _ field: String) throws -> String {
    let cleaned = try requiredCompatibilityString(value, field)
    guard cleaned.hasPrefix("easynet:///r/") else {
        throw invalidCompatibility("\(field) must be a URA")
    }
    return cleaned
}

private func requiredCompatibilityAbilityURA(_ value: String, _ field: String) throws -> String {
    let cleaned = try requiredCompatibilityURA(value, field)
    guard cleaned.contains("/ability/") else {
        throw invalidCompatibility("\(field) must be an Ability URA")
    }
    return cleaned
}

private func requiredCompatibilityInteger(_ value: JSONValue?, _ name: String) throws -> Int64 {
    guard case let .number(number) = value else {
        throw invalidCompatibility("\(name) must be a non-negative integer")
    }
    let integer = Int64(number)
    guard number >= 0, Double(integer) == number else {
        throw invalidCompatibility("\(name) must be a non-negative integer")
    }
    return integer
}

private func requiredCompatibilityBool(_ object: [String: JSONValue], _ name: String) throws -> Bool {
    if case let .bool(value) = object[name] {
        return value
    }
    throw invalidCompatibility("\(name) must be a boolean")
}

private func requiredCompatibilityArray(_ object: [String: JSONValue], _ name: String) throws -> [JSONValue] {
    if case let .array(value) = object[name] {
        return value
    }
    throw invalidCompatibility("\(name) must be an array")
}

private func requiredCompatibilityObject(_ object: [String: JSONValue], _ name: String) throws -> [String: JSONValue] {
    if case let .object(value) = object[name] {
        return value
    }
    throw invalidCompatibility("\(name) must be an object")
}

private func optionalCompatibilityObject(_ value: JSONValue?, _ name: String) throws -> [String: JSONValue]? {
    guard let value else { return nil }
    switch value {
    case .null:
        return nil
    case let .object(object):
        return object
    default:
        throw invalidCompatibility("\(name) must be an object or null")
    }
}

private func validateCompatibilityHash(_ value: String, _ field: String) throws {
    let pattern = #"^sha256:[0-9a-f]{64}$"#
    guard value.range(of: pattern, options: .regularExpression) != nil else {
        throw invalidCompatibility("\(field) must use sha256:<64 lowercase hex> form")
    }
}

private func invalidCompatibility(_ message: String) -> SDKError {
    SDKError(code: .invalidArgument, stage: compatibilityProfile, message: message, details: ["profile": compatibilityProfile])
}

private func compatibilityUnsupported(_ message: String) -> SDKError {
    SDKError(code: .notImplemented, stage: "transport", message: message, details: ["profile": compatibilityProfile])
}
