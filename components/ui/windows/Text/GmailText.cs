using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>The Gmail account's sentences (components/ui/macos/Sentences/GmailText.swift).</summary>
public static class GmailText
{
    public static string Message(this GoogleError error, TimeStyle time) => error switch
    {
        GoogleError.ClientFileUnreadable unreadable => L("Couldn't read the OAuth client file: %@", unreadable.Problem.ClientFileReason()),
        GoogleError.ClientFileInvalid invalid => invalid.Problem.Message(),
        GoogleError.ListenerFailed listener => L("Couldn't start the local sign-in listener: %@", listener.Detail),
        GoogleError.BrowserUnavailable => L("Couldn't open the web browser for Google sign-in."),
        GoogleError.TimedOut timedOut => L("Google sign-in timed out after %@.", time.SpelledOut(timedOut.AfterSeconds)),
        GoogleError.CancelledInBrowser => L("Google sign-in was cancelled in the browser."),
        GoogleError.Denied denied => L("Google sign-in failed (%@).", denied.Code),
        GoogleError.TokenRejected rejected => L("Google rejected the sign-in: %@", rejected.Detail),
        GoogleError.ClientRejected => L("Google rejected the OAuth client. Import the client JSON again."),
        GoogleError.NoRefreshToken => L("Google did not return a refresh token. Sign in again."),
        GoogleError.ScopeNotGranted => L("Gmail access wasn't allowed. Sign in again and allow it."),
        GoogleError.WrongAccount wrong => L("You signed in as %@, but the Gmail address is %@. Sign in with that account.", wrong.Actual, wrong.Expected),
        GoogleError.GmailApiRefused refused => L("The Gmail API refused access (%@). Check that it's enabled in Google Cloud.", refused.Reason),
        GoogleError.Network network => L("Couldn't reach Google: %@", network.Detail),
        GoogleError.SecretStore store => L("Credential Manager error: %@", store.Failure.Description),
        GoogleError.Config config => config.Error.Message(),
        _ => throw new InvalidOperationException($"unknown Google error {error}"),
    };

    public static string Message(this OAuthClientFileProblem problem) => problem switch
    {
        OAuthClientFileProblem.NotJson => L("This file is not a Google OAuth client JSON."),
        OAuthClientFileProblem.WebClient => L("This is a Web application client. Import a Desktop app client JSON."),
        OAuthClientFileProblem.NoDesktopClient => L("This file has no Desktop app client."),
        OAuthClientFileProblem.MissingField missing => L("The OAuth client JSON has a missing or invalid %@ field.", missing.Name),
        OAuthClientFileProblem.InvalidField invalid => L("The OAuth client JSON has a missing or invalid %@ field.", invalid.Name),
        _ => throw new InvalidOperationException($"unknown OAuth client file problem {problem}"),
    };

    /// <summary>Why the OAuth client file the user picked could not be read.</summary>
    public static string ClientFileReason(this FileProblem problem) => problem switch
    {
        FileProblem.NotFound => L("The file does not exist."),
        FileProblem.NotRegularFile => L("This is not a regular file."),
        FileProblem.TooLarge => L("The file is too large to be an OAuth client JSON."),
        FileProblem.Io io => L("%@ failed: %@: %@", io.Operation, io.Path, SettingsFileText.ErrorText(io.Errno)),
        _ => throw new InvalidOperationException($"unknown file problem {problem}"),
    };

    public static string Message(this GoogleConnectBlocker blocker) => blocker switch
    {
        GoogleConnectBlocker.NoOAuthClient => L("No built-in OAuth client. Import a client JSON."),
        GoogleConnectBlocker.EnterAddress => L("Enter the Gmail address."),
        _ => L("Save the Gmail address first."),
    };

    /// <summary>The status line of the OAuth client row.</summary>
    public static string Label(this OAuthClientSource source) => source switch
    {
        OAuthClientSource.BuiltIn => L("Built into the app"),
        OAuthClientSource.Imported imported => L("Imported (%@)", Tail(imported.ClientId)),
        _ => L("Not configured"),
    };

    /// <summary>The end of a client ID without the .apps.googleusercontent.com suffix, such as "…a1b2c3d4e5".</summary>
    private static string Tail(string clientId)
    {
        const string suffix = ".apps.googleusercontent.com";
        var stem = clientId.EndsWith(suffix, StringComparison.Ordinal) ? clientId[..^suffix.Length] : clientId;
        return stem.Length > 10 ? "…" + stem[^10..] : stem;
    }

    /// <summary>The result next to "Test Gmail Connection": its text and whether it succeeded, or null.</summary>
    public static (string Text, bool Succeeded)? Result(this GmailTest test) => test switch
    {
        GmailTest.Idle or GmailTest.Running => null,
        GmailTest.ImapSucceeded => (L("Connection succeeded (IMAP)"), true),
        GmailTest.ApiSucceeded => (L("Connection succeeded (Gmail API)"), true),
        GmailTest.Failed failed => (failed.Failure.Message(), false),
        _ => throw new InvalidOperationException($"unknown Gmail test {test}"),
    };

    public static string Message(this GmailTestFailure failure) => failure switch
    {
        GmailTestFailure.EnterAddress => L("Enter the Gmail address."),
        GmailTestFailure.EnterAppPassword => L("Enter the app password."),
        GmailTestFailure.AppPasswordUnreadable unreadable => L("Couldn't read the app password from Credential Manager: %@", unreadable.Failure.Description),
        GmailTestFailure.ImapLoginRejected { ServerText: { Length: > 0 } text } =>
            L("Gmail rejected the sign-in. Check the address, app password and IMAP setting. (%@)", text),
        GmailTestFailure.ImapLoginRejected => L("Gmail rejected the sign-in. Check the address, app password and IMAP setting."),
        GmailTestFailure.MailboxesMissing missing => MissingMailboxes(missing.AllMail, missing.Trash),
        GmailTestFailure.ImapUnreachable unreachable => L("Could not reach the Gmail server: %@", unreachable.Failure.Message()),
        GmailTestFailure.NoOAuthClient => L("No built-in OAuth client. Import a client JSON."),
        GmailTestFailure.NotSignedIn => L("Sign in with Google first."),
        GmailTestFailure.AccessRevoked => L("Google access was revoked or has expired. Sign in with Google again."),
        GmailTestFailure.ClientRejected => L("Google rejected the OAuth client. Import the client JSON again."),
        GmailTestFailure.SecretStore store => L("Credential Manager error: %@", store.Failure.Description),
        GmailTestFailure.OtherAccount other => L("Connected as %@. Connect again with %@.", other.Authorized, other.Expected),
        GmailTestFailure.AccessTokenRejected => L("Google rejected the access token. Sign in with Google again."),
        GmailTestFailure.GmailApiRefused refused => L("The Gmail API refused access (%@). Check that it's enabled in Google Cloud.", refused.Reason),
        GmailTestFailure.RateLimited => L("The Gmail API rate limit was reached. Try again in a minute."),
        GmailTestFailure.GoogleUnreachable unreachable => L("Couldn't reach Google: %@", unreachable.Detail),
        GmailTestFailure.GoogleReturnedError returned => L("Google returned an error: %@", returned.Detail),
        _ => throw new InvalidOperationException($"unknown Gmail test failure {failure}"),
    };

    private static string MissingMailboxes(bool allMail, bool trash)
    {
        var names = new List<string>();
        if (allMail)
        {
            names.Add(L("All Mail (\\All)"));
        }
        if (trash)
        {
            names.Add(L("Trash (\\Trash)"));
        }
        return L("Mailboxes not visible over IMAP: %@. In Gmail Settings > Labels, turn on 'Show in IMAP'.", string.Join(", ", names));
    }

    public static string Message(this ImapFailure failure) => failure switch
    {
        ImapFailure.Network network => L("Network error (%@)", network.Detail),
        ImapFailure.TimedOut timedOut => L("%@ timed out", timedOut.Step switch
        {
            ImapStep.Command command => command.Name,
            _ => L("Connecting to the server"),
        }),
        ImapFailure.ServerClosed { Text: { } text } => L("The server closed the connection: %@", text),
        ImapFailure.ServerClosed => L("The server closed the connection"),
        ImapFailure.Protocol protocol => L("IMAP protocol error: %@", protocol.Detail),
        ImapFailure.Rejected rejected => L("%@ rejected (%@): %@", rejected.Command, rejected.Status, rejected.Text),
        _ => throw new InvalidOperationException($"unknown IMAP failure {failure}"),
    };

    /// <summary>The pages the Google sign-in listener shows in the browser, in the UI's language.</summary>
    public static SignInPages SignInPages() => new(
        L("FortiVPNManager: you can close this window."),
        L("FortiVPNManager: Google sign-in was not completed. You can close this window."),
        L("FortiVPNManager: this sign-in response is not valid. Start the sign-in again in Settings."));
}
