using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>
/// The account's sentences (components/ui/macos/Sentences/AccountText.swift). The import of the VPN
/// client's settings is macOS only, so its sentences are not here.
/// </summary>
public static class AccountText
{
    public static string Message(this AccountValueProblem problem) => problem switch
    {
        AccountValueProblem.InvalidHost => L("Enter only a host name or IPv4 address (no https://, :port or /)."),
        AccountValueProblem.TooLong tooLong => L("%@ is too long (at most %lld bytes).", tooLong.Field.Name(), tooLong.MaxBytes),
        AccountValueProblem.LeadingOrTrailingSpace space => L("%@ can't start or end with a space.", space.Field.Name()),
        AccountValueProblem.DisallowedCharacters characters => L("%@ contains characters that aren't allowed.", characters.Field.Name()),
        _ => throw new InvalidOperationException($"unknown account value problem {problem}"),
    };

    /// <summary>The field's label; the realm, which has no label of its own in these sentences, by its key.</summary>
    private static string Name(this AccountField field) => field switch
    {
        AccountField.Host => L("Server address"),
        AccountField.Username => L("Username"),
        _ => "realm",
    };

    public static string Message(this AccountSaveError error) => error switch
    {
        AccountSaveError.InvalidPort => L("The port must be a number from 1 to 65535."),
        AccountSaveError.PasswordNeedsAccount => L("Enter the server address and username first."),
        AccountSaveError.AppPasswordNeedsAddress => L("Enter the Gmail address first."),
        AccountSaveError.AccountValue value => value.Problem.Message(),
        AccountSaveError.InvalidVpnPassword password => L("The VPN password can't contain line breaks and must be at most %lld bytes.", password.MaxBytes),
        AccountSaveError.Config config => config.Error.Message(),
        AccountSaveError.SecretStore store => L("Couldn't save the password to Credential Manager: %@", store.Failure.Description),
        _ => throw new InvalidOperationException($"unknown account save error {error}"),
    };

    /// <summary>The prompt of an empty password field.</summary>
    public static string StoredPrompt(bool stored) =>
        stored ? L("Saved in Credential Manager · type to change") : L("Required");
}
