using FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>
/// The account's sentences (components/ui/macos/Sentences/AccountText.swift), with the import's: Windows
/// imports FortiClient's server address.
/// </summary>
public static class AccountText
{
    public static string Message(this AccountValueProblem problem) => problem switch
    {
        AccountValueProblemInvalidHost => L("Enter only a host name or IPv4 address (no https:// or /)."),
        AccountValueProblemTooLong tooLong => L("%@ is too long (at most %lld bytes).", tooLong.Field.Name(), tooLong.MaxBytes),
        AccountValueProblemLeadingOrTrailingSpace space => L("%@ can't start or end with a space.", space.Field.Name()),
        AccountValueProblemDisallowedCharacters characters => L("%@ contains characters that aren't allowed.", characters.Field.Name()),
        _ => throw new InvalidOperationException($"unknown account value problem {problem}"),
    };

    /// <summary>The field's label; the realm, which has no label of its own in these sentences, by its key.</summary>
    private static string Name(this AccountField field) => field switch
    {
        AccountField.Host => L("Server address"),
        AccountField.Username => L("Username"),
        AccountField.Realm => "realm",
        _ => throw new InvalidOperationException($"unknown account field {field}"),
    };

    public static string Message(this AccountSaveError error) => error switch
    {
        AccountSaveErrorInvalidPort => L("The port must be a number from 1 to 65535."),
        AccountSaveErrorPasswordNeedsAccount => L("Enter the server address and username first."),
        AccountSaveErrorAppPasswordNeedsAddress => L("Enter the Gmail address first."),
        AccountSaveErrorAccountValue value => value.Problem.Message(),
        AccountSaveErrorInvalidVpnPassword password => L("The VPN password can't contain line breaks and must be at most %lld bytes.", password.MaxBytes),
        AccountSaveErrorConfig config => config.Error.Message(),
        AccountSaveErrorSecretStore store => L("Couldn't save the password to Credential Manager: %@", store.Failure.Text()),
        _ => throw new InvalidOperationException($"unknown account save error {error}"),
    };

    /// <summary>Why an import took nothing. Windows imports FortiClient's connection; the file cases are macOS's.</summary>
    public static string Message(this ImportError error) => error switch
    {
        ImportErrorNoFortiClientConnection => L("FortiClient has no SSL VPN connection."),
        ImportErrorInvalidFortiClientServer invalid => L("The server address of the FortiClient connection '%@' isn't supported.", invalid.Connection),
        ImportErrorConfig config => config.Error.Message(),
        ImportErrorSourceNotFound notFound => L("openfortivpn config file not found: %@", notFound.Path),
        ImportErrorUnreadable unreadable => L("Could not read the openfortivpn config file (%@): %@", unreadable.Path, unreadable.Problem.ClientFileReason()),
        ImportErrorMissingHost missing => L("The openfortivpn config file has no host: %@", missing.Path),
        _ => throw new InvalidOperationException($"unknown import error {error}"),
    };

    /// <summary>The prompt of an empty password field.</summary>
    public static string StoredPrompt(bool stored) =>
        stored ? L("Saved in Credential Manager · type to change") : L("Required");

    public static string Text(this SecretStoreFailure failure)
    {
        ArgumentNullException.ThrowIfNull(failure);
        return failure.Description ?? $"Unknown error (0x{failure.Code:x})";
    }
}
