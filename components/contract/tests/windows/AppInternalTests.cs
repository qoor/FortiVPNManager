using System.Text.Json;
using FortiVPNManager.Contract.AppInternal;
using Xunit;

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
        List<FromRuntime> messages = RoundTrip<FromRuntime>("from_runtime.json");
        FromRuntimeSection first = Assert.IsType<FromRuntimeSection>(messages[0]);
        ConnectionSection connection = Assert.IsType<SectionConnection>(first.Value).Value;
        _ = Assert.IsType<ConnectionStatusNotConnected>(connection.Status);
        Assert.Equal(ButtonState.Enabled, connection.Connect);
        Assert.Contains(messages, message => message is FromRuntimeOpenSettings);
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
        _ = RoundTrip<FailureReason>("failure_reasons.json");
        _ = RoundTrip<WaitReason>("wait_reasons.json");
        _ = RoundTrip<SetupItem>("setup_items.json");
    }

    [Fact]
    public void ScheduleSections() => RoundTrip<ScheduleSection>("schedule_sections.json");

    [Fact]
    public void ReadinessSections()
    {
        _ = RoundTrip<ReadinessSection>("readiness_sections.json");
        _ = RoundTrip<HelperChannelFailure>("helper_channel_failures.json");
    }

    [Fact]
    public void GmailSections()
    {
        _ = RoundTrip<GmailSection>("gmail_sections.json");
        Assert.Equal(
            [GoogleConnectBlocker.NoOAuthClient, GoogleConnectBlocker.EnterAddress, GoogleConnectBlocker.SaveAddressFirst],
            RoundTrip<GoogleConnectBlocker>("google_connect_blockers.json"));
        _ = RoundTrip<GoogleError>("google_errors.json");
        _ = RoundTrip<GmailTest>("gmail_tests.json");
    }

    [Fact]
    public void BypassRulesSections() => RoundTrip<BypassRulesSection>("bypass_rules_sections.json");

    [Fact]
    public void ConfigSections() => RoundTrip<ConfigSection>("config_sections.json");

    [Fact]
    public void AutoUpdateSections()
    {
        _ = RoundTrip<AutoUpdateSection>("auto_update_sections.json");
        _ = RoundTrip<InstallFailure>("install_failures.json");
        _ = RoundTrip<UpdateCheckFailure>("update_check_failures.json");
    }

    [Fact]
    public void SettingsFileProblems()
    {
        _ = RoundTrip<ConfigFileError>("config_file_errors.json");
        _ = RoundTrip<ConfigProblem>("config_problems.json");
        _ = RoundTrip<ConfigWarning>("config_warnings.json");
        Assert.Equal(
            [
                new AddressProblemMalformed(), new AddressProblemPrefixOutOfRange(4, 8, 32),
                new AddressProblemNonContiguousMask(), new AddressProblemUnroutable(),
                new AddressProblemHostBitsSet("10.30.170.0/24"),
            ],
            RoundTrip<AddressProblem>("address_problems.json"));
        _ = RoundTrip<FileProblem>("file_problems.json");
    }

    [Fact]
    public void PortRequests() => RoundTrip<FromRuntime>("port_requests.json");

    [Fact]
    public void PortActions()
    {
        var actions = RoundTrip<FromRuntime>("port_actions.json")
            .OfType<FromRuntimePortAction>()
            .Select(message => message.Value)
            .ToList();
        Assert.Contains(actions, action => action is PortActionLog { Category: LogCategory.BypassRoutes });
        Assert.Contains(actions, action => action is PortActionLog { Category: LogCategory.LoginItem });
        Assert.Contains(actions, action => action is PortActionLog { Category: LogCategory.Update });
        Assert.Contains(actions, action => action is PortActionReleaseUpdateWaiter { InstallerProcessId: 4_294_967_295 });
    }

    // Sent by the app

    [Fact]
    public void Launched()
    {
        ToRuntimeLaunched launched = Assert.IsType<ToRuntimeLaunched>(Assert.Single(RoundTrip<ToRuntime>("launched.json")));
        Assert.Equal("Asia/Seoul", launched.Facts.TimeZone);
        Assert.NotNull(launched.Facts.BuiltInOauthClient);
    }

    [Fact]
    public void Commands()
    {
        var commands = RoundTrip<ToRuntime>("commands.json")
            .Select(message => Assert.IsType<ToRuntimeCommand>(message).Value)
            .ToList();
        CommandEditAccount edit = Assert.IsType<CommandEditAccount>(commands.Single(command => command is CommandEditAccount));
        Assert.Equal(4UL, edit.Revision);
        Assert.Equal("vpn.example.com:8443", edit.Form.Server);
    }

    [Fact]
    public void PortResponses() => RoundTrip<ToRuntime>("port_responses.json");

    [Fact]
    public void SystemEvents()
    {
        var events = RoundTrip<ToRuntime>("system_events.json")
            .Select(message => Assert.IsType<ToRuntimeEvent>(message).Value)
            .ToList();
        // Whole milliseconds since 1970, as the runtime writes them.
        SystemEventWillSleep willSleep = Assert.IsType<SystemEventWillSleep>(events[0]);
        Assert.Equal(DateTimeOffset.FromUnixTimeMilliseconds(1_762_412_345_250), willSleep.At);
    }

    // Coding

    [Fact]
    public void DecodingIsStrict()
    {
        _ = Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"fly": {}}""", MessageCoding.Default.Command));
        _ = Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"setOpenAtLogin": {"enabled": true, "extra": 1}}""", MessageCoding.Default.Command));
        _ = Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"setOpenAtLogin": {}}""", MessageCoding.Default.Command));
        _ = Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"connect": {}, "disconnect": {}}""", MessageCoding.Default.Command));
        _ = Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"connect": null}""", MessageCoding.Default.Command));
        _ = Assert.Throws<JsonException>(() =>
            JsonSerializer.Deserialize("""{"command": {"_0": {"connect": null}}}""", MessageCoding.Default.ToRuntime));
    }

    [Fact]
    public void DebugOutputLeavesSecretsOut()
    {
        string[] shown =
        [
            new AccountForm("vpn.example.com", "alice", "", "alice@example.com", "vpn-secret", "app-secret").ToString(),
            new HelperCallConnect("""{"password":"vpn-secret"}""").ToString(),
            new HelperCallSubmitOtp("attempt", "123456").ToString(),
            new AppFacts("1", "26.1.0", true, "Asia/Seoul", new SignInPages("a", "b", "c"), "client-secret").ToString(),
        ];
        foreach (string text in shown)
        {
            Assert.DoesNotContain("vpn-secret", text);
            Assert.DoesNotContain("app-secret", text);
            Assert.DoesNotContain("123456", text);
            Assert.DoesNotContain("client-secret", text);
        }
    }
}
