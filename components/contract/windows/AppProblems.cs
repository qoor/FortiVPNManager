namespace FortiVPNManager.Contract;

public static partial class AppInternal
{
    /// <summary>
    /// Problems several sections explain (components/contract/src/app_internal/problem.rs). Entry
    /// indexes start at 0.
    /// </summary>
    [Cases]
    public abstract record ConfigFileError
    {
        private ConfigFileError() { }

        public sealed record InvalidSettings(ConfigProblem Problem) : ConfigFileError;
        public sealed record UnreadableFile(string Path, ConfigProblem Problem) : ConfigFileError;
        public sealed record NotRegularFile(string Path) : ConfigFileError;
        public sealed record DanglingSymlink(string Path) : ConfigFileError;
        public sealed record KeptChanging(string Path) : ConfigFileError;
        public sealed record Io(string Operation, string Path, int Errno) : ConfigFileError;
    }

    public sealed record ConfigProblem(ConfigProblemKind Kind, LineColumn? Location = null);

    /// <summary>1-based; columns count Unicode scalars.</summary>
    public sealed record LineColumn(uint Line, uint Column);

    [Cases]
    public abstract record ConfigProblemKind
    {
        private ConfigProblemKind() { }

        public sealed record FileTooLarge(ulong ByteCount, ulong Limit) : ConfigProblemKind;
        public sealed record InvalidUtf8 : ConfigProblemKind;
        public sealed record DisallowedCharacter(uint Scalar) : ConfigProblemKind;
        public sealed record Syntax(string Problem) : ConfigProblemKind;
        public sealed record MultipleDocuments : ConfigProblemKind;
        public sealed record NestingTooDeep : ConfigProblemKind;
        public sealed record DuplicateKey(string Key) : ConfigProblemKind;
        public sealed record RootNotMapping : ConfigProblemKind;
        public sealed record AnchorOrAlias : ConfigProblemKind;
        public sealed record MergeKey : ConfigProblemKind;
        public sealed record ExplicitTag(string Tag) : ConfigProblemKind;
        public sealed record ComplexKey : ConfigProblemKind;
        public sealed record TypeMismatch(string Path, ValueKind Expected) : ConfigProblemKind;
        public sealed record InvalidInteger(string Path, string Raw) : ConfigProblemKind;
        public sealed record InvalidBoolean(string Path, string Raw) : ConfigProblemKind;
        public sealed record QuotedNumber(string Path, string Raw) : ConfigProblemKind;
        public sealed record QuotedBoolean(string Path, string Raw) : ConfigProblemKind;
        public sealed record InvalidChoice(string Path, string Raw, IReadOnlyList<string> Allowed) : ConfigProblemKind;
        public sealed record OutOfRange(string Path, string Raw, long Minimum, long Maximum) : ConfigProblemKind;
        public sealed record MarginNotLessThanSession(long Margin, long Session) : ConfigProblemKind;
        public sealed record InvalidTime(string Path, string Raw) : ConfigProblemKind;
        public sealed record ScheduleEndNotAfterStart(TimeOfDay Start, TimeOfDay End) : ConfigProblemKind;
        public sealed record InvalidCharacter(string Path, uint Scalar) : ConfigProblemKind;
        public sealed record NewerSchemaVersion(long Version) : ConfigProblemKind;
        public sealed record InvalidRouteEntry(string Path, uint Index, string Raw, AddressProblem Problem) : ConfigProblemKind;
        public sealed record TooManyRouteEntries(string Path, uint Count, uint Limit) : ConfigProblemKind;
        public sealed record DuplicateRouteEntry(string Path, uint Index) : ConfigProblemKind;
        public sealed record OverlappingRouteEntry(string Path, uint Index, string Other) : ConfigProblemKind;
    }

    public enum ValueKind
    {
        Mapping,
        Sequence,
        String,
        Integer,
        Boolean,
    }

    public sealed record ConfigWarning(ConfigWarningKind Kind, LineColumn? Location = null);

    [Cases]
    public abstract record ConfigWarningKind
    {
        private ConfigWarningKind() { }

        public sealed record UnknownKey(string Path) : ConfigWarningKind;
        public sealed record InvalidTrustedCert(uint Index) : ConfigWarningKind;
        public sealed record DuplicateTrustedCert(uint Index) : ConfigWarningKind;
        public sealed record TooManyTrustedCerts(uint Limit) : ConfigWarningKind;
        public sealed record DuplicateRouteEntry(string Path, uint Index) : ConfigWarningKind;
        public sealed record MultipleHardLinks(uint Count) : ConfigWarningKind;
        public sealed record InsecurePermissions(uint Mode) : ConfigWarningKind;
        public sealed record PermissionsCorrected(uint PreviousMode) : ConfigWarningKind;
    }

    [Cases]
    public abstract record AddressProblem
    {
        private AddressProblem() { }

        public sealed record Malformed : AddressProblem;
        public sealed record PrefixOutOfRange(uint Length, uint Minimum, uint Maximum) : AddressProblem;
        public sealed record NonContiguousMask : AddressProblem;
        public sealed record Unroutable : AddressProblem;
        public sealed record HostBitsSet(string Network) : AddressProblem;
    }

    [Cases]
    public abstract record FileProblem
    {
        private FileProblem() { }

        public sealed record NotFound : FileProblem;
        public sealed record NotRegularFile : FileProblem;
        public sealed record TooLarge(ulong LimitBytes) : FileProblem;
        public sealed record Io(string Operation, string Path, int Errno) : FileProblem;
    }

    [Cases]
    public abstract record AccountValueProblem
    {
        private AccountValueProblem() { }

        public sealed record InvalidHost : AccountValueProblem;
        public sealed record TooLong(AccountField Field, uint MaxBytes) : AccountValueProblem;
        public sealed record LeadingOrTrailingSpace(AccountField Field) : AccountValueProblem;
        public sealed record DisallowedCharacters(AccountField Field) : AccountValueProblem;
    }

    public enum AccountField
    {
        Host,
        Username,
        Realm,
    }
}
