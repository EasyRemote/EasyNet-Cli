import Foundation

public indirect enum JSONValue: Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])
}

func decodeObject(_ raw: Data, label: String) throws -> [String: JSONValue] {
    do {
        let decoded = try JSONSerialization.jsonObject(with: raw, options: [])
        guard let object = try jsonValue(decoded).objectValue else {
            throw SDKError.validation("decode", "\(label) must be an object")
        }
        return object
    } catch let error as SDKError {
        throw error
    } catch {
        throw SDKError.validation("decode", "decode \(label): \(error)")
    }
}

func jsonValue(_ value: Any) throws -> JSONValue {
    if value is NSNull {
        return .null
    }
    if let number = value as? NSNumber {
        if CFGetTypeID(number) == CFBooleanGetTypeID() {
            return .bool(number.boolValue)
        }
        return .number(number.doubleValue)
    }
    if let bool = value as? Bool {
        return .bool(bool)
    }
    if let string = value as? String {
        return .string(string)
    }
    if let array = value as? [Any] {
        return .array(try array.map(jsonValue))
    }
    if let object = value as? [String: Any] {
        return .object(try object.mapValues(jsonValue))
    }
    throw SDKError.validation("decode", "JSON contains unsupported value")
}

extension JSONValue {
    var objectValue: [String: JSONValue]? {
        if case let .object(value) = self {
            return value
        }
        return nil
    }
}
