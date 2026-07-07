public struct InvocationTuple: Sendable, Equatable {
    public let caller: String
    public let callee: String
    public let descriptorRef: String
    public let subject: String
    public let nonce: String
    public let causalContext: String
    public let argsJSON: String

    public init(
        caller: String?,
        callee: String?,
        descriptorRef: String?,
        subject: String?,
        nonce: String?,
        causalContext: String?,
        argsJSON: String?
    ) throws {
        self.caller = try InvocationTuple.required(caller, "caller")
        self.callee = try InvocationTuple.required(callee, "callee")
        self.descriptorRef = try InvocationTuple.required(descriptorRef, "descriptorRef")
        self.subject = try InvocationTuple.required(subject, "subject")
        self.nonce = try InvocationTuple.required(nonce, "nonce")
        self.causalContext = try InvocationTuple.required(causalContext, "causalContext")
        self.argsJSON = try InvocationTuple.required(argsJSON, "argsJSON")
    }

    private static func required(_ value: String?, _ field: String) throws -> String {
        guard let value, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw SDKError.validation("invocation", "\(field) is required")
        }
        return value
    }
}

public struct InvocationDraft: Sendable, Equatable {
    public let tuple: InvocationTuple

    public init(tuple: InvocationTuple) {
        self.tuple = tuple
    }

    public func inspectTuple() -> InvocationTuple {
        tuple
    }
}

public final class InvocationBuilder {
    private var caller: String?
    private var callee: String?
    private var descriptorRef: String?
    private var subject: String?
    private var nonce: String?
    private var causalContext: String?
    private var argsJSON: String?

    public init() {}

    @discardableResult
    public func withCallerURA(_ value: String) -> InvocationBuilder {
        caller = value
        return self
    }

    @discardableResult
    public func withCalleeURA(_ value: String) -> InvocationBuilder {
        callee = value
        return self
    }

    @discardableResult
    public func withDescriptorRef(_ value: String) -> InvocationBuilder {
        descriptorRef = value
        return self
    }

    @discardableResult
    public func withSubjectURA(_ value: String) -> InvocationBuilder {
        subject = value
        return self
    }

    @discardableResult
    public func withNonce(_ value: String) -> InvocationBuilder {
        nonce = value
        return self
    }

    @discardableResult
    public func withCausalContext(_ value: String) -> InvocationBuilder {
        causalContext = value
        return self
    }

    @discardableResult
    public func withArgsJSON(_ value: String) -> InvocationBuilder {
        argsJSON = value
        return self
    }

    public func inspect() throws -> InvocationDraft {
        InvocationDraft(
            tuple: try InvocationTuple(
                caller: caller,
                callee: callee,
                descriptorRef: descriptorRef,
                subject: subject,
                nonce: nonce,
                causalContext: causalContext,
                argsJSON: argsJSON
            )
        )
    }

    public func build() throws -> InvocationDraft {
        try inspect()
    }
}
