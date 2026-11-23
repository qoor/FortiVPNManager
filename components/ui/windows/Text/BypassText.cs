using FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>The exception rules' sentences (components/ui/macos/Sentences/BypassText.swift).</summary>
public static class BypassText
{
    /// <summary>"Ethernet · 10.20.161.20/24". Whether the rules apply on it, the status line says.</summary>
    public static string CurrentNetworkText(this BypassRulesSection rules)
    {
        ArgumentNullException.ThrowIfNull(rules);
        if (rules.CurrentNetwork is not { } network)
        {
            return L("Unknown");
        }
        string prefix = network.PrefixLength is { } length ? $"/{length}" : "";
        return $"{network.ServiceName} · {network.Address}{prefix}";
    }

    /// <summary>Set once the retries of a failed write ran out.</summary>
    public static string? StoppedRetryingMessage(this BypassRulesSection rules)
    {
        ArgumentNullException.ThrowIfNull(rules);
        return rules.Status is BypassStatusFailed { StoppedAfterRetries: { } retries }
            ? L("Stopped after %lld failed retries. Tries again when the network or rules change.", retries)
            : null;
    }

    public static (string Text, NoticeStyle Style) Line(this BypassStatus status, TimeStyle time)
    {
        switch (status)
        {
            case BypassStatusChecking:
                return (L("Checking the routes…"), NoticeStyle.Info);
            case BypassStatusNoConfigFile:
                return (L("The settings file doesn't exist yet. Apply creates it."), NoticeStyle.Info);
            case BypassStatusApplied applied:
                var parts = new List<string> { L("Applied to %@ (router %@).", applied.ServiceName, applied.Router) };
                if (applied.OnLinkTargets > 0)
                {
                    parts.Add(L("%lld target(s) are on this network and need no route.", applied.OnLinkTargets));
                }
                return (string.Join(" ", parts), NoticeStyle.Success);
            case BypassStatusVpnOff:
                return (L("Not applied while the VPN is off."), NoticeStyle.Info);
            case BypassStatusOutsideNetworks:
                return (L("Not applied: outside the networks above. Everything goes through the VPN."), NoticeStyle.Info);
            case BypassStatusNoNetworksListed:
                return (L("No networks listed, so the rules apply nowhere."), NoticeStyle.Info);
            case BypassStatusConfigInvalid:
                return (L("The settings file has an error, so the routes are left as they are."), NoticeStyle.Warning);
            case BypassStatusNoNetwork:
                return (L("No network with an IPv4 router. Applies once one connects."), NoticeStyle.Warning);
            case BypassStatusFailed failed:
                string message = failed.Failure.Message();
                return failed.RetryInSeconds is { } seconds
                    ? (message + " " + L("Trying again in %@.", TimeStyle.SpelledOut(seconds)), NoticeStyle.Error)
                    : (message, NoticeStyle.Error);
            default:
                throw new InvalidOperationException($"unknown bypass status {status}");
        }
    }

    public static string Message(this RowProblem problem, string row) => problem switch
    {
        RowProblemInvalid invalid => invalid.Problem.Message(row),
        RowProblemDuplicate duplicate => L("%@ is already in the list.", duplicate.Entry),
        RowProblemOverlap overlap => L("'%@' overlaps %@, which is already in the list.", SettingsFileText.Preview(row), overlap.Other),
        RowProblemVpnServer => L("Can't add the VPN server's address '%@'.", SettingsFileText.Preview(row)),
        _ => throw new InvalidOperationException($"unknown row problem {problem}"),
    };

    /// <summary>The short form beside the row, which already shows the entry.</summary>
    public static string RowMessage(this RowProblem problem) => problem switch
    {
        RowProblemInvalid invalid => invalid.Problem.RowMessage(),
        RowProblemDuplicate => L("This address is already in the list."),
        RowProblemOverlap overlap => L("Overlaps %@.", overlap.Other),
        RowProblemVpnServer => L("The VPN server's address can't be registered as an exception target."),
        _ => throw new InvalidOperationException($"unknown row problem {problem}"),
    };

    /// <summary>The short form beside the row; <c>Message(raw)</c> is the full one.</summary>
    public static string RowMessage(this AddressProblem problem) => problem switch
    {
        AddressProblemMalformed => L("Not an IPv4 address or CIDR block."),
        AddressProblemPrefixOutOfRange range => L("Use a prefix length from /%lld to /%lld.", range.Minimum, range.Maximum),
        AddressProblemNonContiguousMask => L("The subnet mask isn't contiguous."),
        AddressProblemUnroutable => L("Loopback, multicast and reserved addresses can't be routed."),
        AddressProblemHostBitsSet hostBits => L("Did you mean %@?", hostBits.Network),
        _ => throw new InvalidOperationException($"unknown address problem {problem}"),
    };

    public static string Message(this BypassFailure failure) => failure switch
    {
        BypassFailureRoutesNotKept => L("Windows did not keep the routes that were written."),
        BypassFailureNetworkChanged => L("The network or its routes changed while the routes were being written."),
        BypassFailureHelperRefused refused => L("The Helper refused the route list (%@).", refused.Message),
        BypassFailureSettingsNotChanged notChanged => SettingsNotChanged(notChanged.Message),
        BypassFailureHelperUnreachable unreachable => L("Couldn't apply the routes: %@", unreachable.Failure.Message()),
        // A build mismatch has no sentence of its own, only the app's English detail.
        BypassFailureHelperBuildMismatch mismatch => SettingsNotChanged($"Helper build {mismatch.HelperBuild} differs from app build {mismatch.AppBuild}"),
        _ => throw new InvalidOperationException($"unknown bypass failure {failure}"),
    };

    private static string SettingsNotChanged(string detail) => L("Couldn't change the Windows network settings (%@).", detail);

    public const string UpdateAvailableIdentifier = "update-available";

    /// <summary>Notifications of the same kind replace each other.</summary>
    public static string Identifier(this UserNotification notification) => notification switch
    {
        UserNotificationExceptionRulesNotApplied => "exception-rules-write-failed",
        UserNotificationUpdateAvailable => UpdateAvailableIdentifier,
        _ => throw new InvalidOperationException($"unknown notification {notification}"),
    };

    public static string Title(this UserNotification notification) => notification switch
    {
        UserNotificationExceptionRulesNotApplied => L("Exception rules aren't being applied"),
        UserNotificationUpdateAvailable => L("Software Update Available"),
        _ => throw new InvalidOperationException($"unknown notification {notification}"),
    };

    public static string Body(this UserNotification notification) => notification switch
    {
        UserNotificationExceptionRulesNotApplied notApplied =>
            L("Writing the routes still failed after %lld retries: %@", notApplied.Retries, notApplied.Failure.Message()),
        UserNotificationUpdateAvailable available => L("Update to FortiVPNManager %@ to get the latest features.", available.Version),
        _ => throw new InvalidOperationException($"unknown notification {notification}"),
    };

    public static string? ButtonTitle(this UserNotification notification) => notification switch
    {
        UserNotificationExceptionRulesNotApplied => null,
        UserNotificationUpdateAvailable => L("Update Now"),
        _ => throw new InvalidOperationException($"unknown notification {notification}"),
    };
}
