using FortiVPNManager.Contract.AppInternal;
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
        ArgumentNullException.ThrowIfNull(section);
        ArgumentNullException.ThrowIfNull(time);
        switch (section.Status)
        {
            case ConnectionStatusNotConnected:
                return L("Not connected");
            case ConnectionStatusSetupRequired:
                return L("Setup required");
            case ConnectionStatusConnecting connecting:
                string verb = connecting.Reconnecting ? L("Reconnecting") : L("Connecting");
                return verb + " · " + connecting.Progress.Text();
            case ConnectionStatusConnected connected:
                var parts = new List<string> { L("Connected"), TimeStyle.Duration(time.Now - connected.Since) };
                if (connected.ProactiveReconnect is ProactiveReconnectAt at)
                {
                    if (includesPlannedReconnect)
                    {
                        parts.Add(time.PlannedReconnect(at.Time));
                    }
                }
                else if (outlook is ScheduleOutlookSuspended)
                {
                    parts.Add(L("Won't reconnect"));
                }
                else if (outlook is ScheduleOutlookHoldingWhileAway)
                {
                    parts.Add(L("Reconnect on hold"));
                }
                return string.Join(" · ", parts);
            case ConnectionStatusReconnectOnHold:
                return L("Reconnect on hold");
            case ConnectionStatusWaitingToReconnect waiting:
                TimeSpan remaining = waiting.Until - time.Now;
                return remaining > TimeSpan.Zero
                    ? L("Waiting to reconnect (%@)", TimeStyle.Countdown(remaining))
                    : L("Waiting to reconnect");
            case ConnectionStatusWaitingForNetwork:
                return L("Waiting for network");
            case ConnectionStatusDisconnecting disconnecting:
                return disconnecting.Then switch
                {
                    AfterDisconnect.Idle => L("Disconnecting"),
                    AfterDisconnect.Reconnect => L("Reconnecting · Closing current connection"),
                    AfterDisconnect.Sleep => L("Disconnecting before sleep"),
                    AfterDisconnect.Quit => L("Quitting"),
                    _ => throw new InvalidOperationException($"unknown step after disconnecting {disconnecting.Then}"),
                };
            case ConnectionStatusFailed failed:
                return L("Error: %@", failed.Reason.Summary());
            default:
                throw new InvalidOperationException($"unknown status {section.Status}");
        }
    }

    /// <summary>When the next attempt starts, while one is awaited.</summary>
    public static DateTimeOffset? NextAttempt(this ConnectionSection section)
    {
        ArgumentNullException.ThrowIfNull(section);
        return section.Status switch
        {
            ConnectionStatusWaitingToReconnect waiting => waiting.Until,
            ConnectionStatusReconnectOnHold onHold => onHold.Until,
            _ => null,
        };
    }

    public static string Text(this ConnectProgress progress) => progress switch
    {
        ConnectProgress.CheckingReadiness => L("Checking readiness"),
        ConnectProgress.ConnectingToGmail => L("Connecting to Gmail"),
        ConnectProgress.ContactingServer => L("Contacting server"),
        ConnectProgress.LoggingIn => L("Logging in"),
        ConnectProgress.WaitingForOtp => L("Waiting for OTP email"),
        ConnectProgress.VerifyingOtp => L("Verifying OTP"),
        ConnectProgress.OpeningTunnel => L("Opening tunnel"),
        _ => throw new InvalidOperationException($"unknown connect progress {progress}"),
    };

    /// <summary>Short phrase for the status line: "Error: &lt;summary&gt;".</summary>
    public static string Summary(this FailureReason reason) => reason switch
    {
        FailureReasonVpnLoginRejected => L("VPN login rejected"),
        FailureReasonOtpNotReceived { InSpam: true } => L("OTP email is in Spam"),
        FailureReasonOtpNotReceived => L("OTP email not received"),
        FailureReasonOtpRejected => L("OTP verification failed"),
        FailureReasonTunnelSetupFailed => L("Tunnel setup failed"),
        FailureReasonCertificateChangedAfterLogin => L("Server certificate changed"),
        FailureReasonGmailRejected { Method: OtpMethod.Imap } => L("Check Gmail app password"),
        FailureReasonGmailRejected => L("Check Google account connection"),
        FailureReasonTooManyReconnects => L("Too many reconnects"),
        FailureReasonNetwork => L("Network error"),
        FailureReasonHelper helper => helper.Code.Summary(),
        _ => throw new InvalidOperationException($"unknown failure {reason}"),
    };

    /// <summary>One line on what happened, shown with the button that opens Settings.</summary>
    public static string Guidance(this FailureReason reason, TimeStyle time) => reason switch
    {
        FailureReasonVpnLoginRejected => L("The server rejected the login. Check the username and password."),
        FailureReasonOtpNotReceived { InSpam: true } => L("The OTP email is in Spam. Check your Gmail filter."),
        FailureReasonOtpNotReceived otp => L("No OTP email was found within %@.", TimeStyle.SpelledOut(otp.WaitedSeconds)),
        FailureReasonOtpRejected => L("The server did not accept the OTP."),
        FailureReasonTunnelSetupFailed => L("Couldn't open the tunnel after login."),
        FailureReasonCertificateChangedAfterLogin => L("The server certificate was rejected after login."),
        FailureReasonGmailRejected { Method: OtpMethod.Imap } => L("Gmail rejected the login. Check the app password and that IMAP is enabled."),
        FailureReasonGmailRejected => L("Couldn't read Gmail. Connect the Google account again."),
        FailureReasonTooManyReconnects reconnects => L("Automatic reconnection stopped after too many sign-in attempts within %@.", TimeStyle.SpelledOut(reconnects.WindowSeconds)),
        FailureReasonNetwork => L("Couldn't connect to the VPN server."),
        FailureReasonHelper { Code: HelperErrorCode.InvalidRequest } => L("The connection request was rejected. Check the account and VPN password."),
        FailureReasonHelper helper => L("Helper error: %@", helper.Code.Summary()),
        _ => throw new InvalidOperationException($"unknown failure {reason}"),
    };

    /// <summary>
    /// The pane that helps with this failure. An invalid request is an account value or the VPN
    /// password the Helper refused (the log shows which).
    /// </summary>
    public static SettingsPane Pane(this FailureReason reason) => reason switch
    {
        FailureReasonVpnLoginRejected or FailureReasonOtpNotReceived or FailureReasonGmailRejected
            or FailureReasonHelper { Code: HelperErrorCode.InvalidRequest } => SettingsPane.Account,
        FailureReasonHelper => SettingsPane.Advanced,
        _ => SettingsPane.Log,
    };

    /// <summary>The title of the button that opens that pane.</summary>
    public static string ActionTitle(this FailureReason reason) => reason.Pane() switch
    {
        SettingsPane.Account => L("Open Account Settings…"),
        SettingsPane.Advanced => L("Open Advanced Settings…"),
        SettingsPane.Log => L("Show Log"),
        SettingsPane.General or SettingsPane.Connection or SettingsPane.ExceptionRules =>
            throw new InvalidOperationException($"no action for pane {reason.Pane()}"),
        _ => throw new InvalidOperationException($"unknown pane {reason.Pane()}"),
    };

    public static string Summary(this HelperErrorCode code) => code switch
    {
        HelperErrorCode.Busy => L("Helper is busy with another connection"),
        HelperErrorCode.InvalidRequest => L("Invalid connection request"),
        HelperErrorCode.NotAuthorized => L("Helper refused the request"),
        HelperErrorCode.UnknownAttempt or HelperErrorCode.WrongPhase or HelperErrorCode.InternalError => L("Helper internal error"),
        _ => throw new InvalidOperationException($"unknown Helper error code {code}"),
    };

    /// <summary>Why the connection is waiting, shown under the status line.</summary>
    public static string Summary(this WaitReason reason, TimeStyle time) => reason switch
    {
        WaitReasonNetwork => L("Network error"),
        WaitReasonNetworkSetupFailed => L("VPN network setup failed"),
        WaitReasonServerEndedSession => L("Server ended the session"),
        WaitReasonForcedTermination => L("Connection was forcibly ended"),
        WaitReasonAuthFailure failure => L("Authentication failed (%@), retrying once", failure.Reason.Summary()),
        WaitReasonAuthCooldown cooldown => L("Waiting %@ after authentication failure", TimeStyle.SpelledOut(cooldown.DelaySeconds)),
        WaitReasonSleep => L("Resuming after sleep"),
        WaitReasonUserAway => L("Outside the scheduled hours, reconnects when you're back"),
        _ => throw new InvalidOperationException($"unknown wait reason {reason}"),
    };

    /// <summary>The action offered for a setup item, one per item so it is clear what to fix and where.</summary>
    public static string ActionTitle(this SetupItem item) => item switch
    {
        SetupItemAccountDetails => L("Enter Account Details…"),
        SetupItemVpnPassword => L("Enter VPN Password…"),
        SetupItemGmailCredential { Method: OtpMethod.Imap } => L("Enter Gmail App Password…"),
        SetupItemGmailCredential => L("Connect Google Account…"),
        SetupItemConfigFile => L("Check Settings File Error…"),
        // Windows has no approval: the installer registers the Helper service.
        SetupItemHelperApproval => L("Check Helper Service…"),
        SetupItemHelperVersion => L("Check Helper Version…"),
        _ => throw new InvalidOperationException($"unknown setup item {item}"),
    };

    /// <summary>The pane where the item is set up.</summary>
    public static SettingsPane Pane(this SetupItem item) => item switch
    {
        SetupItemAccountDetails or SetupItemVpnPassword or SetupItemGmailCredential => SettingsPane.Account,
        _ => SettingsPane.Advanced,
    };

    public static string AccessibilityText(this ConnectionIcon icon) => icon switch
    {
        ConnectionIcon.Connected => L("Connected"),
        ConnectionIcon.Disconnected => L("Not connected"),
        ConnectionIcon.InProgress => L("Connecting"),
        ConnectionIcon.Error => L("Error"),
        _ => throw new InvalidOperationException($"unknown connection icon {icon}"),
    };
}
