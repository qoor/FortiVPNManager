using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Text;

/// <summary>The exception rules' sentences (components/ui/macos/Sentences/BypassText.swift).</summary>
public static class BypassText
{
    /// <summary>"Ethernet · 10.20.161.20/24". Whether the rules apply on it, the status line says.</summary>
    public static string CurrentNetworkText(this BypassRulesSection rules)
    {
        if (rules.CurrentNetwork is not { } network)
        {
            return L("Unknown");
        }
        var prefix = network.PrefixLength is { } length ? $"/{length}" : "";
        return $"{network.ServiceName} · {network.Address}{prefix}";
    }

    /// <summary>Set once the retries of a failed write ran out.</summary>
    public static string? StoppedRetryingMessage(this BypassRulesSection rules) =>
        rules.Status is BypassStatus.Failed { StoppedAfterRetries: { } retries }
            ? L("Stopped after %lld failed retries. Tries again when the network or rules change.", retries)
            : null;

    public static (string Text, NoticeStyle Style) Line(this BypassStatus status, TimeStyle time)
    {
        switch (status)
        {
            case BypassStatus.Checking:
                return (L("Checking the routes…"), NoticeStyle.Info);
            case BypassStatus.NoConfigFile:
                return (L("The settings file doesn't exist yet. Apply creates it."), NoticeStyle.Info);
            case BypassStatus.Applied applied:
                var parts = new List<string> { L("Applied to %@ (router %@).", applied.ServiceName, applied.Router) };
                if (applied.OnLinkTargets > 0)
                {
                    parts.Add(L("%lld target(s) are on this network and need no route.", applied.OnLinkTargets));
                }
                if (applied.NotImported > 0)
                {
                    parts.Add(L("List full: %lld route(s) added elsewhere weren't imported.", applied.NotImported));
                }
                return (Adding(applied.KeptThroughOtherGateways, parts), NoticeStyle.Success);
            case BypassStatus.OutsideNetworks outside:
                return (Adding(outside.KeptThroughOtherGateways, [L("Not applied: outside the networks above. Everything goes through the VPN.")]), NoticeStyle.Info);
            case BypassStatus.NoNetworksListed none:
                return (Adding(none.KeptThroughOtherGateways, [L("No networks listed, so the rules apply nowhere.")]), NoticeStyle.Info);
            case BypassStatus.ConfigInvalid:
                return (L("The settings file has an error, so the routes are left as they are."), NoticeStyle.Warning);
            case BypassStatus.NoNetwork:
                return (L("No network with an IPv4 router. Applies once one connects."), NoticeStyle.Warning);
            case BypassStatus.Failed failed:
                var message = failed.Failure.Message();
                return failed.RetryInSeconds is { } seconds
                    ? (message + " " + L("Trying again in %@.", time.SpelledOut(seconds)), NoticeStyle.Error)
                    : (message, NoticeStyle.Error);
            default:
                throw new InvalidOperationException($"unknown bypass status {status}");
        }
    }

    private static string Adding(uint kept, List<string> parts)
    {
        if (kept > 0)
        {
            parts.Add(L("%lld route(s) through other gateways are left as they are.", kept));
        }
        return string.Join(" ", parts);
    }

    public static string Message(this RowProblem problem, string row) => problem switch
    {
        RowProblem.Invalid invalid => invalid.Problem.Message(row),
        RowProblem.Duplicate duplicate => L("%@ is already in the list.", duplicate.Entry),
        RowProblem.Overlap overlap => L("'%@' overlaps %@, which is already in the list.", SettingsFileText.Preview(row), overlap.Other),
        _ => throw new InvalidOperationException($"unknown row problem {problem}"),
    };

    /// <summary>The short form beside the row, which already shows the entry.</summary>
    public static string RowMessage(this RowProblem problem) => problem switch
    {
        RowProblem.Invalid invalid => invalid.Problem.RowMessage(),
        RowProblem.Duplicate => L("This address is already in the list."),
        RowProblem.Overlap overlap => L("Overlaps %@.", overlap.Other),
        _ => throw new InvalidOperationException($"unknown row problem {problem}"),
    };

    /// <summary>The short form beside the row; <c>Message(raw)</c> is the full one.</summary>
    public static string RowMessage(this AddressProblem problem) => problem switch
    {
        AddressProblem.Malformed => L("Not an IPv4 address or CIDR block."),
        AddressProblem.PrefixOutOfRange range => L("Use a prefix length from /%lld to /%lld.", range.Minimum, range.Maximum),
        AddressProblem.NonContiguousMask => L("The subnet mask isn't contiguous."),
        AddressProblem.Unroutable => L("Loopback, multicast and reserved addresses can't be routed."),
        AddressProblem.HostBitsSet hostBits => L("Did you mean %@?", hostBits.Network),
        _ => throw new InvalidOperationException($"unknown address problem {problem}"),
    };

    public static string Message(this BypassFailure failure) => failure switch
    {
        BypassFailure.RoutesNotKept => L("Windows did not keep the routes that were written."),
        BypassFailure.NetworkChanged => L("The network or its routes changed while the routes were being written."),
        BypassFailure.HelperRefused refused => L("The Helper refused the route list (%@).", refused.Message),
        BypassFailure.SettingsNotChanged notChanged => SettingsNotChanged(notChanged.Message),
        BypassFailure.ImportNotSaved notSaved => L("Couldn't save the imported routes to the settings file: %@", notSaved.Error.Message()),
        BypassFailure.HelperUnreachable unreachable => L("Couldn't apply the routes: %@", unreachable.Failure.Message()),
        // A build mismatch has no sentence of its own, only the app's English detail.
        BypassFailure.HelperBuildMismatch mismatch => SettingsNotChanged($"Helper build {mismatch.HelperBuild} differs from app build {mismatch.AppBuild}"),
        _ => throw new InvalidOperationException($"unknown bypass failure {failure}"),
    };

    private static string SettingsNotChanged(string detail) => L("Couldn't change the Windows network settings (%@).", detail);

    /// <summary>Notifications of the same kind replace each other.</summary>
    public static string Identifier(this UserNotification notification) => notification switch
    {
        UserNotification.ExceptionRulesNotApplied => "exception-rules-write-failed",
        _ => throw new InvalidOperationException($"unknown notification {notification}"),
    };

    public static string Title(this UserNotification notification) => notification switch
    {
        UserNotification.ExceptionRulesNotApplied => L("Exception rules aren't being applied"),
        _ => throw new InvalidOperationException($"unknown notification {notification}"),
    };

    public static string Body(this UserNotification notification) => notification switch
    {
        UserNotification.ExceptionRulesNotApplied notApplied =>
            L("Writing the routes still failed after %lld retries: %@", notApplied.Retries, notApplied.Failure.Message()),
        _ => throw new InvalidOperationException($"unknown notification {notification}"),
    };
}
