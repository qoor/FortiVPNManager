using FortiVPNManager.Contract;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>
/// The connection's sentences, shared by the menu and the General pane so both describe the same
/// state with the same words (components/ui/macos/Sentences/ConnectionText.swift).
/// </summary>
public static class ConnectionText
{
    /// <summary>
    /// One line: "Connected · 1 hr 23 min · Reconnects in about 1 hr 32 min", "Waiting to reconnect (1:12)".
    /// <paramref name="outlook"/> is the schedule's, which ends the line of a session without a
    /// planned reconnect. Without <paramref name="includesPlannedReconnect"/>, a planned reconnect
    /// leaves the line to a row of its own.
    /// </summary>
    public static string StatusLine(this ConnectionSection section, ScheduleOutlook? outlook, TimeStyle time, bool includesPlannedReconnect = true)
    {
        switch (section.Status)
        {
            case ConnectionStatus.NotConnected:
                return L("Not connected");
            case ConnectionStatus.SetupRequired:
                return L("Setup required");
            case ConnectionStatus.Connecting connecting:
                var verb = connecting.Reconnecting ? L("Reconnecting") : L("Connecting");
                return verb + " · " + connecting.Progress.Text();
            case ConnectionStatus.Connected connected:
                var parts = new List<string> { L("Connected"), time.Duration(time.Now - connected.Since) };
                if (connected.ProactiveReconnect is ProactiveReconnect.At at)
                {
                    if (includesPlannedReconnect)
                    {
                        parts.Add(time.PlannedReconnect(at.Time));
                    }
                }
                else if (outlook is ScheduleOutlook.Suspended)
                {
                    parts.Add(L("Won't reconnect"));
                }
                else if (outlook is ScheduleOutlook.HoldingWhileAway)
                {
                    parts.Add(L("Reconnect on hold"));
                }
                return string.Join(" · ", parts);
            case ConnectionStatus.ReconnectOnHold:
                return L("Reconnect on hold");
            case ConnectionStatus.WaitingToReconnect waiting:
                var remaining = waiting.Until - time.Now;
                return remaining > TimeSpan.Zero
                    ? L("Waiting to reconnect (%@)", TimeStyle.Countdown(remaining))
                    : L("Waiting to reconnect");
            case ConnectionStatus.WaitingForNetwork:
                return L("Waiting for network");
            case ConnectionStatus.Disconnecting disconnecting:
                return disconnecting.Then switch
                {
                    AfterDisconnect.Idle => L("Disconnecting"),
                    AfterDisconnect.Reconnect => L("Reconnecting · Closing current connection"),
                    AfterDisconnect.Sleep => L("Disconnecting before sleep"),
                    _ => L("Quitting"),
                };
            case ConnectionStatus.Failed failed:
                return L("Error: %@", failed.Reason.Summary());
            default:
                throw new InvalidOperationException($"unknown status {section.Status}");
        }
    }

    /// <summary>When the next attempt starts, while one is awaited.</summary>
    public static DateTimeOffset? NextAttempt(this ConnectionSection section) => section.Status switch
    {
        ConnectionStatus.WaitingToReconnect waiting => waiting.Until,
        ConnectionStatus.ReconnectOnHold onHold => onHold.Until,
        _ => null,
    };

    public static string Text(this ConnectProgress progress) => progress switch
    {
        ConnectProgress.CheckingReadiness => L("Checking readiness"),
        ConnectProgress.ConnectingToGmail => L("Connecting to Gmail"),
        ConnectProgress.ContactingServer => L("Contacting server"),
        ConnectProgress.LoggingIn => L("Logging in"),
        ConnectProgress.WaitingForOtp => L("Waiting for OTP email"),
        ConnectProgress.VerifyingOtp => L("Verifying OTP"),
        _ => L("Opening tunnel"),
    };

    /// <summary>Short phrase for the status line: "Error: &lt;summary&gt;".</summary>
    public static string Summary(this FailureReason reason) => reason switch
    {
        FailureReason.VpnLoginRejected => L("VPN login rejected"),
        FailureReason.OtpNotReceived { InSpam: true } => L("OTP email is in Spam"),
        FailureReason.OtpNotReceived => L("OTP email not received"),
        FailureReason.OtpRejected => L("OTP verification failed"),
        FailureReason.TunnelSetupFailed => L("Tunnel setup failed"),
        FailureReason.TunnelModeRejected => L("Server rejected tunnel mode"),
        FailureReason.DroppedEarly => L("Dropped right after connecting"),
        FailureReason.CertificateChangedAfterLogin => L("Server certificate changed"),
        FailureReason.GmailRejected { Method: OtpMethod.Imap } => L("Check Gmail app password"),
        FailureReason.GmailRejected => L("Check Google account connection"),
        FailureReason.TooManyReconnects => L("Too many reconnects"),
        FailureReason.Network => L("Network error"),
        FailureReason.Helper helper => helper.Code.Summary(),
        _ => throw new InvalidOperationException($"unknown failure {reason}"),
    };

    /// <summary>One line on what happened, shown with the button that opens Settings.</summary>
    public static string Guidance(this FailureReason reason, TimeStyle time) => reason switch
    {
        FailureReason.VpnLoginRejected => L("The server rejected the login. Check the username and password."),
        FailureReason.OtpNotReceived { InSpam: true } => L("The OTP email is in Spam. Check your Gmail filter."),
        FailureReason.OtpNotReceived otp => L("No OTP email was found within %@.", time.SpelledOut(otp.WaitedSeconds)),
        FailureReason.OtpRejected => L("The server did not accept the OTP."),
        FailureReason.TunnelSetupFailed => L("Couldn't open the tunnel after login."),
        FailureReason.TunnelModeRejected => L("The gateway doesn't allow SSL VPN tunnel mode."),
        FailureReason.DroppedEarly dropped => L("The connection dropped within %@.", time.SpelledOut(dropped.WithinSeconds)),
        FailureReason.CertificateChangedAfterLogin => L("The server certificate was rejected after login."),
        FailureReason.GmailRejected { Method: OtpMethod.Imap } => L("Gmail rejected the login. Check the app password and that IMAP is enabled."),
        FailureReason.GmailRejected => L("Couldn't read Gmail. Connect the Google account again."),
        FailureReason.TooManyReconnects reconnects => L("Automatic reconnection stopped after too many sign-in attempts within %@.", time.SpelledOut(reconnects.WindowSeconds)),
        FailureReason.Network => L("Couldn't connect to the VPN server."),
        FailureReason.Helper { Code: HelperErrorCode.InvalidRequest } => L("The connection request was rejected. Check the account and VPN password."),
        FailureReason.Helper helper => L("Helper error: %@", helper.Code.Summary()),
        _ => throw new InvalidOperationException($"unknown failure {reason}"),
    };

    /// <summary>
    /// The pane that helps with this failure. An invalid request is an account value or the VPN
    /// password the Helper refused (the log shows which).
    /// </summary>
    public static SettingsPane Pane(this FailureReason reason) => reason switch
    {
        FailureReason.VpnLoginRejected or FailureReason.OtpNotReceived or FailureReason.GmailRejected
            or FailureReason.Helper { Code: HelperErrorCode.InvalidRequest } => SettingsPane.Account,
        FailureReason.Helper => SettingsPane.Advanced,
        _ => SettingsPane.Log,
    };

    /// <summary>The title of the button that opens that pane.</summary>
    public static string ActionTitle(this FailureReason reason) => reason.Pane() switch
    {
        SettingsPane.Account => L("Open Account Settings…"),
        SettingsPane.Advanced => L("Open Advanced Settings…"),
        _ => L("Show Log"),
    };

    public static string Summary(this HelperErrorCode code) => code switch
    {
        HelperErrorCode.Busy => L("Helper is busy with another connection"),
        HelperErrorCode.InvalidRequest => L("Invalid connection request"),
        HelperErrorCode.ClientUnavailable => L("openconnect not found"),
        HelperErrorCode.NotAuthorized => L("Helper refused the request"),
        _ => L("Helper internal error"),
    };

    /// <summary>Why the connection is waiting, shown under the status line.</summary>
    public static string Summary(this WaitReason reason, TimeStyle time) => reason switch
    {
        WaitReason.Network => L("Network error"),
        WaitReason.ServerEndedSession => L("Server ended the session"),
        WaitReason.ForcedTermination => L("Connection was forcibly ended"),
        WaitReason.AuthFailure failure => L("Authentication failed (%@), retrying once", failure.Reason.Summary()),
        WaitReason.AuthCooldown cooldown => L("Waiting %@ after authentication failure", time.SpelledOut(cooldown.DelaySeconds)),
        WaitReason.Sleep => L("Resuming after sleep"),
        WaitReason.UserAway => L("Outside the scheduled hours, reconnects when you're back"),
        _ => throw new InvalidOperationException($"unknown wait reason {reason}"),
    };

    /// <summary>The action offered for a setup item, one per item so it is clear what to fix and where.</summary>
    public static string ActionTitle(this SetupItem item) => item switch
    {
        SetupItem.AccountDetails => L("Enter Account Details…"),
        SetupItem.VpnPassword => L("Enter VPN Password…"),
        SetupItem.GmailCredential { Method: OtpMethod.Imap } => L("Enter Gmail App Password…"),
        SetupItem.GmailCredential => L("Connect Google Account…"),
        SetupItem.ConfigFile => L("Check Settings File Error…"),
        // Windows has no approval: the install script registers the Helper service.
        SetupItem.HelperApproval => L("Check Helper Service…"),
        SetupItem.HelperVersion => L("Check Helper Version…"),
        _ => throw new InvalidOperationException($"unknown setup item {item}"),
    };

    /// <summary>The pane where the item is set up.</summary>
    public static SettingsPane Pane(this SetupItem item) => item switch
    {
        SetupItem.AccountDetails or SetupItem.VpnPassword or SetupItem.GmailCredential => SettingsPane.Account,
        _ => SettingsPane.Advanced,
    };

    public static string AccessibilityText(this ConnectionIcon icon) => icon switch
    {
        ConnectionIcon.Connected => L("Connected"),
        ConnectionIcon.Disconnected => L("Not connected"),
        ConnectionIcon.InProgress => L("Connecting"),
        _ => L("Error"),
    };
}
