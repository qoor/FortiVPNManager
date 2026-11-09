extension AppInternal {
    /// Problems several sections explain (components/contract/src/app_internal/problem.rs).
    /// Entry indexes start at 0.
    enum ConfigFileError: Decodable, Equatable, Sendable {
        case invalidSettings(problem: ConfigProblem)
        case unreadableFile(path: String, problem: ConfigProblem)
        case notRegularFile(path: String)
        case danglingSymlink(path: String)
        case keptChanging(path: String)
        case io(operation: String, path: String, errno: Int32)
    }

    struct ConfigProblem: Decodable, Equatable, Sendable {
        var kind: ConfigProblemKind
        var location: LineColumn?
    }

    /// 1-based; columns count Unicode scalars.
    struct LineColumn: Decodable, Equatable, Sendable {
        var line: Int
        var column: Int
    }

    enum ConfigProblemKind: Decodable, Equatable, Sendable {
        case fileTooLarge(byteCount: Int, limit: Int)
        case invalidUtf8
        case disallowedCharacter(scalar: UInt32)
        case syntax(problem: String)
        case multipleDocuments
        case nestingTooDeep
        case duplicateKey(key: String)
        case rootNotMapping
        case anchorOrAlias
        case mergeKey
        case explicitTag(tag: String)
        case complexKey
        case typeMismatch(path: String, expected: ValueKind)
        case invalidInteger(path: String, raw: String)
        case invalidBoolean(path: String, raw: String)
        case quotedNumber(path: String, raw: String)
        case quotedBoolean(path: String, raw: String)
        case invalidChoice(path: String, raw: String, allowed: [String])
        case outOfRange(path: String, raw: String, minimum: Int, maximum: Int)
        case marginNotLessThanSession(margin: Int, session: Int)
        case invalidTime(path: String, raw: String)
        case scheduleEndNotAfterStart(start: TimeOfDay, end: TimeOfDay)
        case invalidCharacter(path: String, scalar: UInt32)
        case newerSchemaVersion(version: Int)
        case invalidRouteEntry(path: String, index: Int, raw: String, problem: AddressProblem)
        case tooManyRouteEntries(path: String, count: Int, limit: Int)
        case duplicateRouteEntry(path: String, index: Int)
        case overlappingRouteEntry(path: String, index: Int, other: String)
    }

    enum ValueKind: String, Decodable, Sendable {
        case mapping
        case sequence
        case string
        case integer
        case boolean
    }

    struct ConfigWarning: Decodable, Equatable, Sendable {
        var kind: ConfigWarningKind
        var location: LineColumn?
    }

    enum ConfigWarningKind: Decodable, Equatable, Sendable {
        case unknownKey(path: String)
        case invalidTrustedCert(index: Int)
        case duplicateTrustedCert(index: Int)
        case duplicateRouteEntry(path: String, index: Int)
        case multipleHardLinks(count: Int)
        case insecurePermissions(mode: UInt32)
        case permissionsCorrected(previousMode: UInt32)
    }

    enum AddressProblem: Decodable, Equatable, Sendable {
        case malformed
        case prefixOutOfRange(length: Int, minimum: Int, maximum: Int)
        case nonContiguousMask
        case unroutable
        case hostBitsSet(network: String)
    }

    enum FileProblem: Decodable, Equatable, Sendable {
        case notFound
        case notRegularFile
        case tooLarge(limitBytes: Int)
        case io(operation: String, path: String, errno: Int32)
    }

    enum AccountValueProblem: Decodable, Equatable, Sendable {
        case invalidHost
        case tooLong(field: AccountField, maxBytes: Int)
        case leadingOrTrailingSpace(field: AccountField)
        case disallowedCharacters(field: AccountField)
    }

    enum AccountField: String, Decodable, Sendable {
        case host
        case username
        case realm
    }
}
