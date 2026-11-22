using System.Text.Json;
using Xunit;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.Contract.Tests;

/// <summary>
/// The C# types in components/contract/windows against ../fixtures/app-internal: what the runtime
/// sends decodes, and what the app sends encodes, into the JSON the Rust tests pin.
/// </summary>
public class AppInternalTests
{
    private static List<T> RoundTrip<T>(string name) => Fixtures.RoundTrip<T>($"app-internal/{name}");

    // Sent by the runtime

    [Fact]
    public void MessagesFromTheRuntime()
    {
        var messages = RoundTrip<FromRuntime>("from_runtime.json");
        var first = Assert.IsType<FromRuntime.Section>(messages[0]);
        var connection = Assert.IsType<Section.Connection>(first.Value).Value;
        Assert.IsType<ConnectionStatus.NotConnected>(connection.Status);
        Assert.Equal(ButtonState.Enabled, connection.Connect);
        Assert.Contains(messages, message => message is FromRuntime.OpenSettings);
    }

    [Fact]
    public void ConnectionSections() => RoundTrip<ConnectionSection>("connection_sections.json");

    [Fact]
    public void ConnectionKinds()
    {
        Assert.Equal(
            [
                ConnectProgress.CheckingReadiness, ConnectProgress.ConnectingToGmail, ConnectProgress.ContactingServer,
                ConnectProgress.LoggingIn, ConnectProgress.WaitingForOtp, ConnectProgress.VerifyingOtp,
                ConnectProgress.OpeningTunnel,
            ],
            RoundTrip<ConnectProgress>("connect_progress.json"));
        Assert.Equal(
            [AfterDisconnect.Idle, AfterDisconnect.Reconnect, AfterDisconnect.Sleep, AfterDisconnect.Quit],
            RoundTrip<AfterDisconnect>("after_disconnect.json"));
        RoundTrip<FailureReason>("failure_reasons.json");
        RoundTrip<WaitReason>("wait_reasons.json");
        RoundTrip<SetupItem>("setup_items.json");
    }

    [Fact]
    public void ScheduleSections() => RoundTrip<ScheduleSection>("schedule_sections.json");

    [Fact]
    public void ReadinessSections()
    {
        RoundTrip<ReadinessSection>("readiness_sections.json");
        RoundTrip<HelperChannelFailure>("helper_channel_failures.json");
    }

    [Fact]
    public void GmailSections()
    {
        RoundTrip<GmailSection>("gmail_sections.json");
        Assert.Equal(
            [GoogleConnectBlocker.NoOAuthClient, GoogleConnectBlocker.EnterAddress, GoogleConnectBlocker.SaveAddressFirst],
            RoundTrip<GoogleConnectBlocker>("google_connect_blockers.json"));
        RoundTrip<GoogleError>("google_errors.json");
        RoundTrip<GmailTest>("gmail_tests.json");
    }

    [Fact]
    public void BypassRulesSections() => RoundTrip<BypassRulesSection>("bypass_rules_sections.json");

    [Fact]
    public void ConfigSections() => RoundTrip<ConfigSection>("config_sections.json");

    [Fact]
    public void AutoUpdateSections()
    {
        RoundTrip<AutoUpdateSection>("auto_update_sections.json");
        RoundTrip<InstallFailure>("install_failures.json");
        RoundTrip<UpdateCheckFailure>("update_check_failures.json");
    }

    [Fact]
    public void SettingsFileProblems()
    {
        RoundTrip<ConfigFileError>("config_file_errors.json");
        RoundTrip<ConfigProblem>("config_problems.json");
        RoundTrip<ConfigWarning>("config_warnings.json");
        Assert.Equal(
            [
                new AddressProblem.Malformed(), new AddressProblem.PrefixOutOfRange(4, 8, 32),
                new AddressProblem.NonContiguousMask(), new AddressProblem.Unroutable(),
                new AddressProblem.HostBitsSet("10.30.170.0/24"),
            ],
            RoundTrip<AddressProblem>("address_problems.json"));
        RoundTrip<FileProblem>("file_problems.json");
    }

    [Fact]
    public void PortRequests() => RoundTrip<FromRuntime>("port_requests.json");

    [Fact]
    public void PortActions()
    {
        var actions = RoundTrip<FromRuntime>("port_actions.json")
            .OfType<FromRuntime.PortAction>()
            .Select(message => message.Value)
            .ToList();
        Assert.Contains(actions, action => action is PortAction.Log { Category: LogCategory.BypassRoutes });
        Assert.Contains(actions, action => action is PortAction.Log { Category: LogCategory.LoginItem });
        Assert.Contains(actions, action => action is PortAction.Log { Category: LogCategory.Update });
        Assert.Contains(actions, action => action is PortAction.ReleaseUpdateWaiter { InstallerProcessId: 4_294_967_295 });
    }

    // Sent by the app

    [Fact]
    public void Launched()
    {
        var launched = Assert.IsType<ToRuntime.Launched>(Assert.Single(RoundTrip<ToRuntime>("launched.json")));
        Assert.Equal("Asia/Seoul", launched.Facts.TimeZone);
        Assert.NotNull(launched.Facts.BuiltInOauthClient);
    }

    [Fact]
    public void Commands()
    {
        var commands = RoundTrip<ToRuntime>("commands.json")
            .Select(message => Assert.IsType<ToRuntime.Command>(message).Value)
            .ToList();
        var edit = Assert.IsType<Command.EditAccount>(commands.Single(command => command is Command.EditAccount));
        Assert.Equal(4UL, edit.Revision);
        Assert.Equal("vpn.example.com:8443", edit.Form.Server);
    }

    [Fact]
    public void PortResponses() => RoundTrip<ToRuntime>("port_responses.json");

    [Fact]
    public void SystemEvents()
    {
        var events = RoundTrip<ToRuntime>("system_events.json")
            .Select(message => Assert.IsType<ToRuntime.Event>(message).Value)
            .ToList();
        // Whole milliseconds since 1970, as the runtime writes them.
        var willSleep = Assert.IsType<SystemEvent.WillSleep>(events[0]);
        Assert.Equal(DateTimeOffset.FromUnixTimeMilliseconds(1_762_412_345_250), willSleep.At);
    }

    // Coding

    [Fact]
    public void DecodingIsStrict()
    {
        Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"fly": {}}""", MessageCoding.Default.Command));
        Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"setOpenAtLogin": {"enabled": true, "extra": 1}}""", MessageCoding.Default.Command));
        Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"setOpenAtLogin": {}}""", MessageCoding.Default.Command));
        Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"connect": {}, "disconnect": {}}""", MessageCoding.Default.Command));
        Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"connect": null}""", MessageCoding.Default.Command));
        Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"command": {"_0": {"connect": null}}}""", MessageCoding.Default.ToRuntime));
    }

    [Fact]
    public void DebugOutputLeavesSecretsOut()
    {
        string[] shown =
        [
            new AccountForm("vpn.example.com", "alice", "", "alice@example.com", "vpn-secret", "app-secret").ToString(),
            new HelperCall.Connect("""{"password":"vpn-secret"}""").ToString(),
            new HelperCall.SubmitOtp("attempt", "123456").ToString(),
            new AppFacts("1", "26.1.0", true, "Asia/Seoul", new SignInPages("a", "b", "c"), "client-secret").ToString(),
        ];
        foreach (var text in shown)
        {
            Assert.DoesNotContain("vpn-secret", text);
            Assert.DoesNotContain("app-secret", text);
            Assert.DoesNotContain("123456", text);
            Assert.DoesNotContain("client-secret", text);
        }
    }
}
