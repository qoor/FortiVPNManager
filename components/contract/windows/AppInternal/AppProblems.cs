using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract.AppInternal;

/// <summary>
/// Problems several sections explain (components/contract/src/app_internal/problem.rs). Entry
/// indexes start at 0.
/// </summary>
[JsonConverter(typeof(CasesConverter<ConfigFileError>))]
[JsonCase(typeof(ConfigFileErrorInvalidSettings), "invalidSettings")]
[JsonCase(typeof(ConfigFileErrorUnreadableFile), "unreadableFile")]
[JsonCase(typeof(ConfigFileErrorNotRegularFile), "notRegularFile")]
[JsonCase(typeof(ConfigFileErrorDanglingSymlink), "danglingSymlink")]
[JsonCase(typeof(ConfigFileErrorKeptChanging), "keptChanging")]
[JsonCase(typeof(ConfigFileErrorIo), "io")]
public abstract record ConfigFileError
{
    private protected ConfigFileError() { }
}

public sealed record ConfigFileErrorInvalidSettings(ConfigProblem Problem) : ConfigFileError;

public sealed record ConfigFileErrorUnreadableFile(string Path, ConfigProblem Problem) : ConfigFileError;

public sealed record ConfigFileErrorNotRegularFile(string Path) : ConfigFileError;

public sealed record ConfigFileErrorDanglingSymlink(string Path) : ConfigFileError;

public sealed record ConfigFileErrorKeptChanging(string Path) : ConfigFileError;

public sealed record ConfigFileErrorIo(string Operation, string Path, int Errno) : ConfigFileError;

public sealed record ConfigProblem(ConfigProblemKind Kind, LineColumn? Location = null);

/// <summary>1-based; columns count Unicode scalars.</summary>
public sealed record LineColumn(uint Line, uint Column);

[JsonConverter(typeof(CasesConverter<ConfigProblemKind>))]
[JsonCase(typeof(ConfigProblemKindFileTooLarge), "fileTooLarge")]
[JsonCase(typeof(ConfigProblemKindInvalidUtf8), "invalidUtf8")]
[JsonCase(typeof(ConfigProblemKindDisallowedCharacter), "disallowedCharacter")]
[JsonCase(typeof(ConfigProblemKindSyntax), "syntax")]
[JsonCase(typeof(ConfigProblemKindMultipleDocuments), "multipleDocuments")]
[JsonCase(typeof(ConfigProblemKindNestingTooDeep), "nestingTooDeep")]
[JsonCase(typeof(ConfigProblemKindDuplicateKey), "duplicateKey")]
[JsonCase(typeof(ConfigProblemKindRootNotMapping), "rootNotMapping")]
[JsonCase(typeof(ConfigProblemKindAnchorOrAlias), "anchorOrAlias")]
[JsonCase(typeof(ConfigProblemKindMergeKey), "mergeKey")]
[JsonCase(typeof(ConfigProblemKindExplicitTag), "explicitTag")]
[JsonCase(typeof(ConfigProblemKindComplexKey), "complexKey")]
[JsonCase(typeof(ConfigProblemKindTypeMismatch), "typeMismatch")]
[JsonCase(typeof(ConfigProblemKindInvalidInteger), "invalidInteger")]
[JsonCase(typeof(ConfigProblemKindInvalidBoolean), "invalidBoolean")]
[JsonCase(typeof(ConfigProblemKindQuotedNumber), "quotedNumber")]
[JsonCase(typeof(ConfigProblemKindQuotedBoolean), "quotedBoolean")]
[JsonCase(typeof(ConfigProblemKindInvalidChoice), "invalidChoice")]
[JsonCase(typeof(ConfigProblemKindOutOfRange), "outOfRange")]
[JsonCase(typeof(ConfigProblemKindMarginNotLessThanSession), "marginNotLessThanSession")]
[JsonCase(typeof(ConfigProblemKindInvalidTime), "invalidTime")]
[JsonCase(typeof(ConfigProblemKindScheduleEndNotAfterStart), "scheduleEndNotAfterStart")]
[JsonCase(typeof(ConfigProblemKindInvalidCharacter), "invalidCharacter")]
[JsonCase(typeof(ConfigProblemKindNewerSchemaVersion), "newerSchemaVersion")]
[JsonCase(typeof(ConfigProblemKindInvalidRouteEntry), "invalidRouteEntry")]
[JsonCase(typeof(ConfigProblemKindTooManyRouteEntries), "tooManyRouteEntries")]
[JsonCase(typeof(ConfigProblemKindDuplicateRouteEntry), "duplicateRouteEntry")]
[JsonCase(typeof(ConfigProblemKindOverlappingRouteEntry), "overlappingRouteEntry")]
[JsonCase(typeof(ConfigProblemKindVpnServerAsRouteTarget), "vpnServerAsRouteTarget")]
public abstract record ConfigProblemKind
{
    private protected ConfigProblemKind() { }
}

public sealed record ConfigProblemKindFileTooLarge(ulong ByteCount, ulong Limit) : ConfigProblemKind;

public sealed record ConfigProblemKindInvalidUtf8 : ConfigProblemKind;

public sealed record ConfigProblemKindDisallowedCharacter(uint Scalar) : ConfigProblemKind;

public sealed record ConfigProblemKindSyntax(string Problem) : ConfigProblemKind;

public sealed record ConfigProblemKindMultipleDocuments : ConfigProblemKind;

public sealed record ConfigProblemKindNestingTooDeep : ConfigProblemKind;

public sealed record ConfigProblemKindDuplicateKey(string Key) : ConfigProblemKind;

public sealed record ConfigProblemKindRootNotMapping : ConfigProblemKind;

public sealed record ConfigProblemKindAnchorOrAlias : ConfigProblemKind;

public sealed record ConfigProblemKindMergeKey : ConfigProblemKind;

public sealed record ConfigProblemKindExplicitTag(string Tag) : ConfigProblemKind;

public sealed record ConfigProblemKindComplexKey : ConfigProblemKind;

public sealed record ConfigProblemKindTypeMismatch(string Path, ValueKind Expected) : ConfigProblemKind;

public sealed record ConfigProblemKindInvalidInteger(string Path, string Raw) : ConfigProblemKind;

public sealed record ConfigProblemKindInvalidBoolean(string Path, string Raw) : ConfigProblemKind;

public sealed record ConfigProblemKindQuotedNumber(string Path, string Raw) : ConfigProblemKind;

public sealed record ConfigProblemKindQuotedBoolean(string Path, string Raw) : ConfigProblemKind;

public sealed record ConfigProblemKindInvalidChoice(string Path, string Raw, IReadOnlyList<string> Allowed) : ConfigProblemKind;

public sealed record ConfigProblemKindOutOfRange(string Path, string Raw, long Minimum, long Maximum) : ConfigProblemKind;

public sealed record ConfigProblemKindMarginNotLessThanSession(long Margin, long Session) : ConfigProblemKind;

public sealed record ConfigProblemKindInvalidTime(string Path, string Raw) : ConfigProblemKind;

public sealed record ConfigProblemKindScheduleEndNotAfterStart(TimeOfDay Start, TimeOfDay End) : ConfigProblemKind;

public sealed record ConfigProblemKindInvalidCharacter(string Path, uint Scalar) : ConfigProblemKind;

public sealed record ConfigProblemKindNewerSchemaVersion(long Version) : ConfigProblemKind;

public sealed record ConfigProblemKindInvalidRouteEntry(string Path, uint Index, string Raw, AddressProblem Problem) : ConfigProblemKind;

public sealed record ConfigProblemKindTooManyRouteEntries(string Path, uint Count, uint Limit) : ConfigProblemKind;

public sealed record ConfigProblemKindDuplicateRouteEntry(string Path, uint Index) : ConfigProblemKind;

public sealed record ConfigProblemKindOverlappingRouteEntry(string Path, uint Index, string Other) : ConfigProblemKind;

public sealed record ConfigProblemKindVpnServerAsRouteTarget(string Path) : ConfigProblemKind;

[JsonConverter(typeof(EnumNameConverter<ValueKind>))]
public enum ValueKind
{
    Mapping,
    Sequence,
    String,
    Integer,
    Boolean,
}

public sealed record ConfigWarning(ConfigWarningKind Kind, LineColumn? Location = null);

[JsonConverter(typeof(CasesConverter<ConfigWarningKind>))]
[JsonCase(typeof(ConfigWarningKindUnknownKey), "unknownKey")]
[JsonCase(typeof(ConfigWarningKindInvalidTrustedCert), "invalidTrustedCert")]
[JsonCase(typeof(ConfigWarningKindDuplicateTrustedCert), "duplicateTrustedCert")]
[JsonCase(typeof(ConfigWarningKindTooManyTrustedCerts), "tooManyTrustedCerts")]
[JsonCase(typeof(ConfigWarningKindDuplicateRouteEntry), "duplicateRouteEntry")]
[JsonCase(typeof(ConfigWarningKindMultipleHardLinks), "multipleHardLinks")]
[JsonCase(typeof(ConfigWarningKindInsecurePermissions), "insecurePermissions")]
[JsonCase(typeof(ConfigWarningKindPermissionsCorrected), "permissionsCorrected")]
public abstract record ConfigWarningKind
{
    private protected ConfigWarningKind() { }
}

public sealed record ConfigWarningKindUnknownKey(string Path) : ConfigWarningKind;

public sealed record ConfigWarningKindInvalidTrustedCert(uint Index) : ConfigWarningKind;

public sealed record ConfigWarningKindDuplicateTrustedCert(uint Index) : ConfigWarningKind;

public sealed record ConfigWarningKindTooManyTrustedCerts(uint Limit) : ConfigWarningKind;

public sealed record ConfigWarningKindDuplicateRouteEntry(string Path, uint Index) : ConfigWarningKind;

public sealed record ConfigWarningKindMultipleHardLinks(uint Count) : ConfigWarningKind;

public sealed record ConfigWarningKindInsecurePermissions(uint Mode) : ConfigWarningKind;

public sealed record ConfigWarningKindPermissionsCorrected(uint PreviousMode) : ConfigWarningKind;

[JsonConverter(typeof(CasesConverter<AddressProblem>))]
[JsonCase(typeof(AddressProblemMalformed), "malformed")]
[JsonCase(typeof(AddressProblemPrefixOutOfRange), "prefixOutOfRange")]
[JsonCase(typeof(AddressProblemNonContiguousMask), "nonContiguousMask")]
[JsonCase(typeof(AddressProblemUnroutable), "unroutable")]
[JsonCase(typeof(AddressProblemHostBitsSet), "hostBitsSet")]
public abstract record AddressProblem
{
    private protected AddressProblem() { }
}

public sealed record AddressProblemMalformed : AddressProblem;

public sealed record AddressProblemPrefixOutOfRange(uint Length, uint Minimum, uint Maximum) : AddressProblem;

public sealed record AddressProblemNonContiguousMask : AddressProblem;

public sealed record AddressProblemUnroutable : AddressProblem;

public sealed record AddressProblemHostBitsSet(string Network) : AddressProblem;

[JsonConverter(typeof(CasesConverter<FileProblem>))]
[JsonCase(typeof(FileProblemNotFound), "notFound")]
[JsonCase(typeof(FileProblemNotRegularFile), "notRegularFile")]
[JsonCase(typeof(FileProblemTooLarge), "tooLarge")]
[JsonCase(typeof(FileProblemIo), "io")]
public abstract record FileProblem
{
    private protected FileProblem() { }
}

public sealed record FileProblemNotFound : FileProblem;

public sealed record FileProblemNotRegularFile : FileProblem;

public sealed record FileProblemTooLarge(ulong LimitBytes) : FileProblem;

public sealed record FileProblemIo(string Operation, string Path, int Errno) : FileProblem;

[JsonConverter(typeof(CasesConverter<AccountValueProblem>))]
[JsonCase(typeof(AccountValueProblemInvalidHost), "invalidHost")]
[JsonCase(typeof(AccountValueProblemTooLong), "tooLong")]
[JsonCase(typeof(AccountValueProblemLeadingOrTrailingSpace), "leadingOrTrailingSpace")]
[JsonCase(typeof(AccountValueProblemDisallowedCharacters), "disallowedCharacters")]
public abstract record AccountValueProblem
{
    private protected AccountValueProblem() { }
}

public sealed record AccountValueProblemInvalidHost : AccountValueProblem;

public sealed record AccountValueProblemTooLong(AccountField Field, uint MaxBytes) : AccountValueProblem;

public sealed record AccountValueProblemLeadingOrTrailingSpace(AccountField Field) : AccountValueProblem;

public sealed record AccountValueProblemDisallowedCharacters(AccountField Field) : AccountValueProblem;

[JsonConverter(typeof(EnumNameConverter<AccountField>))]
public enum AccountField
{
    Host,
    Username,
    Realm,
}
