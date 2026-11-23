using FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>The Gmail account's sentences (components/ui/macos/Sentences/GmailText.swift).</summary>
public static class GmailText
{
    public static string Message(this GoogleError error, TimeStyle time) => error switch
    {
        GoogleErrorClientFileUnreadable unreadable => L("Couldn't read the OAuth client file: %@", unreadable.Problem.ClientFileReason()),
        GoogleErrorClientFileInvalid invalid => invalid.Problem.Message(),
        GoogleErrorListenerFailed listener => L("Couldn't start the local sign-in listener: %@", listener.Detail),
        GoogleErrorBrowserUnavailable => L("Couldn't open the web browser for Google sign-in."),
        GoogleErrorTimedOut timedOut => L("Google sign-in timed out after %@.", TimeStyle.SpelledOut(timedOut.AfterSeconds)),
        GoogleErrorCancelledInBrowser => L("Google sign-in was cancelled in the browser."),
        GoogleErrorDenied denied => L("Google sign-in failed (%@).", denied.Code),
        GoogleErrorTokenRejected rejected => L("Google rejected the sign-in: %@", rejected.Detail),
        GoogleErrorClientRejected => L("Google rejected the OAuth client. Import the client JSON again."),
        GoogleErrorNoRefreshToken => L("Google did not return a refresh token. Sign in again."),
        GoogleErrorScopeNotGranted => L("Gmail access wasn't allowed. Sign in again and allow it."),
        GoogleErrorWrongAccount wrong => L("You signed in as %@, but the Gmail address is %@. Sign in with that account.", wrong.Actual, wrong.Expected),
        GoogleErrorGmailApiRefused refused => L("The Gmail API refused access (%@). Check that it's enabled in Google Cloud.", refused.Reason),
        GoogleErrorNetwork network => L("Couldn't reach Google: %@", network.Detail),
        GoogleErrorSecretStore store => L("Credential Manager error: %@", store.Failure.Text()),
        GoogleErrorConfig config => config.Error.Message(),
        _ => throw new InvalidOperationException($"unknown Google error {error}"),
    };

    public static string Message(this OAuthClientFileProblem problem) => problem switch
    {
        OAuthClientFileProblemNotJson => L("This file is not a Google OAuth client JSON."),
        OAuthClientFileProblemWebClient => L("This is a Web application client. Import a Desktop app client JSON."),
        OAuthClientFileProblemNoDesktopClient => L("This file has no Desktop app client."),
        OAuthClientFileProblemMissingField missing => L("The OAuth client JSON has a missing or invalid %@ field.", missing.Name),
        OAuthClientFileProblemInvalidField invalid => L("The OAuth client JSON has a missing or invalid %@ field.", invalid.Name),
        _ => throw new InvalidOperationException($"unknown OAuth client file problem {problem}"),
    };

    /// <summary>Why the OAuth client file the user picked could not be read.</summary>
    public static string ClientFileReason(this FileProblem problem) => problem switch
    {
        FileProblemNotFound => L("The file does not exist."),
        FileProblemNotRegularFile => L("This is not a regular file."),
        FileProblemTooLarge => L("The file is too large to be an OAuth client JSON."),
        FileProblemIo io => L("%@ failed: %@: %@", io.Operation, io.Path, SettingsFileText.ErrorText(io.Errno)),
        _ => throw new InvalidOperationException($"unknown file problem {problem}"),
    };

    public static string Message(this GoogleConnectBlocker blocker) => blocker switch
    {
        GoogleConnectBlocker.NoOAuthClient => L("No built-in OAuth client. Import a client JSON."),
        GoogleConnectBlocker.EnterAddress => L("Enter the Gmail address."),
        GoogleConnectBlocker.SaveAddressFirst => L("Save the Gmail address first."),
        _ => throw new InvalidOperationException($"unknown Google connect blocker {blocker}"),
    };

    /// <summary>The status line of the OAuth client row.</summary>
    public static string Label(this OAuthClientSource source) => source switch
    {
        OAuthClientSourceBuiltIn => L("Built into the app"),
        OAuthClientSourceImported imported => L("Imported (%@)", Tail(imported.ClientId)),
        _ => L("Not configured"),
    };

    /// <summary>The end of a client ID without the .apps.googleusercontent.com suffix, such as "…a1b2c3d4e5".</summary>
    private static string Tail(string clientId)
    {
        const string suffix = ".apps.googleusercontent.com";
        string stem = clientId.EndsWith(suffix, StringComparison.Ordinal) ? clientId[..^suffix.Length] : clientId;
        return stem.Length > 10 ? "…" + stem[^10..] : stem;
    }

    /// <summary>The result next to "Test Gmail Connection": its text and whether it succeeded, or null.</summary>
    public static (string Text, bool Succeeded)? Result(this GmailTest test) => test switch
    {
        GmailTestIdle or GmailTestRunning => null,
        GmailTestImapSucceeded => (L("Connection succeeded (IMAP)"), true),
        GmailTestApiSucceeded => (L("Connection succeeded (Gmail API)"), true),
        GmailTestFailed failed => (failed.Failure.Message(), false),
        _ => throw new InvalidOperationException($"unknown Gmail test {test}"),
    };

    public static string Message(this GmailTestFailure failure) => failure switch
    {
        GmailTestFailureEnterAddress => L("Enter the Gmail address."),
        GmailTestFailureEnterAppPassword => L("Enter the app password."),
        GmailTestFailureAppPasswordUnreadable unreadable => L("Couldn't read the app password from Credential Manager: %@", unreadable.Failure.Text()),
        GmailTestFailureImapLoginRejected { ServerText: { Length: > 0 } text } =>
            L("Gmail rejected the sign-in. Check the address, app password and IMAP setting. (%@)", text),
        GmailTestFailureImapLoginRejected => L("Gmail rejected the sign-in. Check the address, app password and IMAP setting."),
        GmailTestFailureMailboxesMissing missing => MissingMailboxes(missing.AllMail, missing.Trash),
        GmailTestFailureImapUnreachable unreachable => L("Could not reach the Gmail server: %@", unreachable.Failure.Message()),
        GmailTestFailureNoOAuthClient => L("No built-in OAuth client. Import a client JSON."),
        GmailTestFailureNotSignedIn => L("Sign in with Google first."),
        GmailTestFailureAccessRevoked => L("Google access was revoked or has expired. Sign in with Google again."),
        GmailTestFailureClientRejected => L("Google rejected the OAuth client. Import the client JSON again."),
        GmailTestFailureSecretStore store => L("Credential Manager error: %@", store.Failure.Text()),
        GmailTestFailureOtherAccount other => L("Connected as %@. Connect again with %@.", other.Authorized, other.Expected),
        GmailTestFailureAccessTokenRejected => L("Google rejected the access token. Sign in with Google again."),
        GmailTestFailureGmailApiRefused refused => L("The Gmail API refused access (%@). Check that it's enabled in Google Cloud.", refused.Reason),
        GmailTestFailureRateLimited => L("The Gmail API rate limit was reached. Try again in a minute."),
        GmailTestFailureGoogleUnreachable unreachable => L("Couldn't reach Google: %@", unreachable.Detail),
        GmailTestFailureGoogleReturnedError returned => L("Google returned an error: %@", returned.Detail),
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
        ImapFailureNetwork network => L("Network error (%@)", network.Detail),
        ImapFailureTimedOut timedOut => L("%@ timed out", timedOut.Step switch
        {
            ImapStepCommand command => command.Name,
            _ => L("Connecting to the server"),
        }),
        ImapFailureServerClosed { Text: { } text } => L("The server closed the connection: %@", text),
        ImapFailureServerClosed => L("The server closed the connection"),
        ImapFailureProtocol protocol => L("IMAP protocol error: %@", protocol.Detail),
        ImapFailureRejected rejected => L("%@ rejected (%@): %@", rejected.Command, rejected.Status, rejected.Text),
        _ => throw new InvalidOperationException($"unknown IMAP failure {failure}"),
    };

    /// <summary>The pages the Google sign-in listener shows in the browser, in the UI's language.</summary>
    public static SignInPages SignInPages() => new(
        L("FortiVPNManager: you can close this window."),
        L("FortiVPNManager: Google sign-in was not completed. You can close this window."),
        L("FortiVPNManager: this sign-in response is not valid. Start the sign-in again in Settings."));
}
