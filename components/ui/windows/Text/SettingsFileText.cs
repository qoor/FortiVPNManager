using System.ComponentModel;
using System.Globalization;
using System.Text;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>
/// Sentences about the settings file (config.yaml) and IPv4 entries
/// (components/ui/macos/Sentences/SettingsFileText.swift). Paths such as vpn.port come as written in
/// the file; the few rules that name fixed keys spell them here.
/// </summary>
public static class SettingsFileText
{
    private const string TrustedCerts = "vpn.trusted_certs";
    private const string SessionMaxMinutes = "connection.session_max_minutes";
    private const string MarginMinutes = "connection.proactive_reconnect.margin_minutes";
    private const string ScheduleStart = "connection.schedule.start";
    private const string ScheduleEnd = "connection.schedule.end";

    public static string Message(this ConfigFileError error) => error switch
    {
        ConfigFileError.InvalidSettings invalid => L("The settings to save are invalid. %@", invalid.Problem.Kind.Message()),
        ConfigFileError.UnreadableFile unreadable => L("Can't read the config file (%@). %@", unreadable.Path, unreadable.Problem.DisplayMessage()),
        ConfigFileError.NotRegularFile notRegular => L("The config file path (%@) is not a regular file.", notRegular.Path),
        ConfigFileError.DanglingSymlink dangling => L("The config file (%@) is a symbolic link to a target that doesn't exist.", dangling.Path),
        ConfigFileError.KeptChanging changing => L("The config file (%@) kept changing while it was being saved, so it wasn't saved. Try again.", changing.Path),
        ConfigFileError.Io io => L("Config file operation (%@) failed: %@: %@", io.Operation, io.Path, ErrorText(io.Errno)),
        _ => throw new InvalidOperationException($"unknown config file error {error}"),
    };

    /// <summary>The message, after the location when it is known: "Line 3, column 9: …".</summary>
    public static string DisplayMessage(this ConfigProblem problem) =>
        problem.Location is { } location ? location.Description() + ": " + problem.Kind.Message() : problem.Kind.Message();

    public static string DisplayMessage(this ConfigWarning warning) =>
        warning.Location is { } location ? location.Description() + ": " + warning.Kind.Message() : warning.Kind.Message();

    public static string Description(this LineColumn location) => L("Line %lld, column %lld", location.Line, location.Column);

    public static string Message(this ConfigProblemKind kind) => kind switch
    {
        ConfigProblemKind.FileTooLarge large => L("The config file is too large (%lld bytes). It must be %lld KiB (%lld bytes) or smaller.", large.ByteCount, large.Limit / 1024, large.Limit),
        ConfigProblemKind.InvalidUtf8 => L("The config file is not valid UTF-8 text. Save it as UTF-8."),
        ConfigProblemKind.DisallowedCharacter character => L("The config file contains a character that is not allowed (%@). Remove that character.", CodePoint(character.Scalar)),
        ConfigProblemKind.Syntax syntax => L("YAML syntax error (%@).", syntax.Problem),
        ConfigProblemKind.MultipleDocuments => L("The config file must contain only one YAML document. Remove the document separator (---)."),
        ConfigProblemKind.NestingTooDeep => L("The YAML structure is nested too deeply."),
        ConfigProblemKind.DuplicateKey duplicate => L("The key '%@' appears more than once at the same level. Keep only one.", duplicate.Key),
        ConfigProblemKind.RootNotMapping => L("The top level of the config file must be in 'key: value' form."),
        ConfigProblemKind.AnchorOrAlias => L("Anchors (&) and aliases (*) are not allowed. Write the values out directly."),
        ConfigProblemKind.MergeKey => L("Merge keys (<<) are not allowed. Write the values out directly."),
        ConfigProblemKind.ExplicitTag tag => L("YAML tags (%@) are not allowed. Write just the value, without a tag.", tag.Tag),
        ConfigProblemKind.ComplexKey => L("Keys must be plain strings."),
        ConfigProblemKind.TypeMismatch mismatch => L("The value of %@ has the wrong type. %@.", mismatch.Path, mismatch.Expected.Requirement()),
        ConfigProblemKind.InvalidInteger integer => string.Join(" ",
            L("%@ is set to '%@', which is not an accepted number format.", integer.Path, Preview(integer.Raw)),
            L("Write it as a decimal number without leading zeros (e.g. %@: 443).", Leaf(integer.Path)),
            L("Formats such as 0443 (octal), 2:30 (base 60), 0x1BB (hex) and 1_000 are not accepted.")),
        ConfigProblemKind.InvalidBoolean boolean => L("%@ is set to '%@', which is not allowed. Use true or false.", boolean.Path, Preview(boolean.Raw)),
        ConfigProblemKind.QuotedNumber number => string.Join(" ",
            L("%@ has a quoted value \"%@\".", number.Path, Preview(number.Raw)),
            L("Write numbers without quotes (e.g. %@: 443).", Leaf(number.Path))),
        ConfigProblemKind.QuotedBoolean boolean => string.Join(" ",
            L("%@ has a quoted value \"%@\".", boolean.Path, Preview(boolean.Raw)),
            L("Write true/false without quotes (e.g. %@: true).", Leaf(boolean.Path))),
        ConfigProblemKind.InvalidChoice choice => L("%@ is set to '%@', which is not allowed. Use one of: %@.", choice.Path, Preview(choice.Raw), string.Join(", ", choice.Allowed)),
        ConfigProblemKind.OutOfRange { Minimum: var minimum, Maximum: var maximum } range when minimum == maximum =>
            L("%@ is set to %@, which is not allowed. It must be %lld.", range.Path, Preview(range.Raw), minimum),
        ConfigProblemKind.OutOfRange range => L("%@ is set to %@, which is outside the allowed range (%lld–%lld).", range.Path, Preview(range.Raw), range.Minimum, range.Maximum),
        ConfigProblemKind.MarginNotLessThanSession margin => L("%@ (%lld) must be less than %@ (%lld).", MarginMinutes, margin.Margin, SessionMaxMinutes, margin.Session),
        ConfigProblemKind.InvalidTime time => L("%@ is set to '%@', which is not a time. Write it as \"HH:mm\" between 00:00 and 23:59 (e.g. %@: \"09:00\").", time.Path, Preview(time.Raw), Leaf(time.Path)),
        ConfigProblemKind.ScheduleEndNotAfterStart hours => L("%@ (%@) must be later than %@ (%@).", ScheduleEnd, hours.End.Description(), ScheduleStart, hours.Start.Description()),
        ConfigProblemKind.InvalidCharacter character => string.Join(" ",
            L("%@ contains a character that is not allowed (%@).", character.Path, CodePoint(character.Scalar)),
            L("Control characters, line breaks, BOMs and similar characters are not allowed.")),
        ConfigProblemKind.NewerSchemaVersion version => string.Join(" ",
            L("This config file was written by a newer version of the app (schema_version %lld).", version.Version),
            L("Update the app. Until then, this file won't be overwritten.")),
        ConfigProblemKind.InvalidRouteEntry entry => L("In %@, entry %lld: %@", entry.Path, entry.Index + 1, entry.Problem.Message(entry.Raw)),
        ConfigProblemKind.TooManyRouteEntries tooMany => L("%@ has %lld entries. It can have at most %lld.", tooMany.Path, tooMany.Count, tooMany.Limit),
        ConfigProblemKind.DuplicateRouteEntry duplicate => L("In %@, entry %lld repeats an earlier entry.", duplicate.Path, duplicate.Index + 1),
        ConfigProblemKind.OverlappingRouteEntry overlap => L("In %@, entry %lld overlaps %@, which is already listed. Entries in one list must not overlap.", overlap.Path, overlap.Index + 1, overlap.Other),
        _ => throw new InvalidOperationException($"unknown config problem {kind}"),
    };

    private static string Requirement(this ValueKind kind) => kind switch
    {
        ValueKind.Mapping => L("It must be nested entries in 'key: value' form"),
        ValueKind.Sequence => L("It must be a list (e.g. [] or '- value' lines)"),
        ValueKind.String => L("It must be a string"),
        ValueKind.Integer => L("It must be an unquoted integer"),
        _ => L("It must be an unquoted true or false"),
    };

    public static string Message(this ConfigWarningKind kind) => kind switch
    {
        ConfigWarningKind.UnknownKey unknown => L("Ignoring unknown key '%@'. It will be removed the next time you save in Settings.", unknown.Path),
        ConfigWarningKind.InvalidTrustedCert invalid => L("In %@, entry %lld is not a SHA-256 digest (64 hex digits) and is ignored.", TrustedCerts, invalid.Index + 1),
        ConfigWarningKind.DuplicateTrustedCert duplicate => L("In %@, entry %lld duplicates an earlier entry and is ignored.", TrustedCerts, duplicate.Index + 1),
        ConfigWarningKind.TooManyTrustedCerts tooMany => L("%@ keeps at most %lld entries; the entries after them are ignored.", TrustedCerts, tooMany.Limit),
        ConfigWarningKind.DuplicateRouteEntry duplicate => L("In %@, entry %lld duplicates an earlier entry and is ignored.", duplicate.Path, duplicate.Index + 1),
        ConfigWarningKind.MultipleHardLinks links => L("The config file has %lld hard links. Saving won't update the other links.", links.Count),
        ConfigWarningKind.InsecurePermissions insecure => L("The config file permissions are %@. They will be changed to 0600 on the next save.", Octal(insecure.Mode)),
        ConfigWarningKind.PermissionsCorrected corrected => L("Changed the config file permissions from %@ to 0600.", Octal(corrected.PreviousMode)),
        _ => throw new InvalidOperationException($"unknown config warning {kind}"),
    };

    /// <summary>The explanation for the entry <paramref name="raw"/>, as typed or written in the file.</summary>
    public static string Message(this AddressProblem problem, string raw)
    {
        var text = Preview(raw);
        return problem switch
        {
            AddressProblem.Malformed => L("'%@' is not an IPv4 address or network. Write it like 10.30.161.16 or 10.30.170.0/24.", text),
            AddressProblem.PrefixOutOfRange range => L("'%@' has the prefix length /%lld. Use /%lld to /%lld.", text, range.Length, range.Minimum, range.Maximum),
            AddressProblem.NonContiguousMask => L("'%@' has a subnet mask that is not contiguous.", text),
            AddressProblem.Unroutable => L("'%@' is a loopback, multicast or reserved address and can't be routed.", text),
            AddressProblem.HostBitsSet hostBits => L("'%@' has bits set after the prefix. Did you mean %@?", text, hostBits.Network),
            _ => throw new InvalidOperationException($"unknown address problem {problem}"),
        };
    }

    /// <summary>HH:mm, as the settings file writes it.</summary>
    public static string Description(this TimeOfDay time) =>
        string.Create(CultureInfo.InvariantCulture, $"{time.Hour:00}:{time.Minute:00}");

    /// <summary>The system's description of an error code the runtime reported for a file operation.</summary>
    public static string ErrorText(int code) => new Win32Exception(code).Message;

    /// <summary>At most 40 characters of user text on one line, for messages.</summary>
    public static string Preview(string raw)
    {
        const int limit = 40;
        var text = new StringBuilder();
        var count = 0;
        foreach (var rune in raw.EnumerateRunes())
        {
            if (count == limit)
            {
                text.Append('…');
                break;
            }
            text.Append(IsDisallowedInText(rune) ? "�" : rune.ToString());
            count++;
        }
        return text.ToString();
    }

    /// <summary>"U+0007".</summary>
    private static string CodePoint(uint value) => "U+" + value.ToString("X4", CultureInfo.InvariantCulture);

    /// <summary>Permission bits as four octal digits: "0644".</summary>
    private static string Octal(uint mode) => Convert.ToString(mode & 0xFFF, 8).PadLeft(4, '0');

    /// <summary>The last component of a dotted key path.</summary>
    private static string Leaf(string path) => path.Split('.')[^1];

    /// <summary>Control characters, line and paragraph separators, the BOM and noncharacters.</summary>
    private static bool IsDisallowedInText(Rune rune) =>
        rune.Value is 0x2028 or 0x2029 or 0xFEFF
        || Rune.GetUnicodeCategory(rune) == UnicodeCategory.Control
        || (rune.Value >= 0xFDD0 && rune.Value <= 0xFDEF)
        || (rune.Value & 0xFFFE) == 0xFFFE;
}
