using CommunityToolkit.WinUI.Controls;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The General pane (components/ui/macos/GeneralPane.swift): the connection's status with its
/// buttons, as in the menu, and how the app starts. Importing the VPN client's settings is macOS only.
/// </summary>
internal sealed class GeneralPane : IPane
{
    private readonly AppUi ui;
    private readonly StackPanel status = new() { Spacing = 4 };
    private readonly StackPanel startup = new() { Spacing = 4 };
    private readonly SettingsCard current = Cards.Card(L("Current status"));
    private readonly FontIcon currentMark;
    private readonly TextBlock currentText;
    private readonly TextBlock reasonValue = Cards.Value();
    private readonly SettingsCard reason;
    private readonly StackPanel setupItems = new() { Spacing = 4 };
    private readonly InfoBar failure = Cards.Notice(InfoBarSeverity.Error);
    private readonly Button failureAction = new();
    private readonly TextBlock connectedAtValue = Cards.Value();
    private readonly TextBlock tunnelIpValue = Cards.Value();
    private readonly TextBlock proactiveValue = Cards.Value();
    private readonly TextBlock nextAttemptValue = Cards.Value();
    private readonly SettingsCard connectedAt;
    private readonly SettingsCard tunnelIp;
    private readonly SettingsCard proactive;
    private readonly SettingsCard nextAttempt;
    private readonly Button connect = new() { Content = L("Connect") };
    private readonly Button disconnect = new() { Content = L("Disconnect") };
    private readonly Button reconnectNow = new() { Content = L("Reconnect Now") };
    private readonly ToggleSwitch openAtLogin = new();
    private readonly SettingsCard openAtLoginCard;
    private readonly InfoBar openAtLoginError = Cards.Notice(InfoBarSeverity.Error);
    private readonly ToggleSwitch autoConnect = new();
    private readonly SettingsCard autoConnectCard;
    private IReadOnlyList<SetupItem> shownItems = [];
    private FailureReason? shownFailure;
    private bool refreshing;

    public GeneralPane(AppUi ui, PanePage page)
    {
        this.ui = ui;
        (var currentLine, currentMark, currentText) = Cards.StatusLine();
        current.Content = currentLine;
        reason = Cards.Card(L("Reason"), reasonValue);
        connectedAt = Cards.Card(L("Connected at"), connectedAtValue);
        tunnelIp = Cards.Card(L("Tunnel IP"), tunnelIpValue);
        proactive = Cards.Card(L("Next proactive reconnect"), proactiveValue);
        nextAttempt = Cards.Card(L("Next attempt"), nextAttemptValue);
        failure.ActionButton = failureAction;
        failureAction.Click += (_, _) =>
        {
            if (shownFailure is { } shown)
            {
                ui.Settings.Show(shown.Pane());
            }
        };
        connect.Click += (_, _) => ui.Send(new Command.Connect());
        disconnect.Click += (_, _) => ui.Send(new Command.Disconnect());
        reconnectNow.Click += (_, _) => ui.Send(new Command.ReconnectNow());
        status.Children.Add(Cards.SectionHeader(L("Status"), first: true));
        foreach (var element in new UIElement[] { current, reason, setupItems, failure, connectedAt, tunnelIp, proactive, nextAttempt, Cards.ActionCard(Cards.Buttons(connect, disconnect, reconnectNow)) })
        {
            status.Children.Add(element);
        }

        openAtLoginCard = Cards.Card(L("Open at login"), openAtLogin);
        openAtLogin.Toggled += (_, _) =>
        {
            if (!refreshing)
            {
                ui.Send(new Command.SetOpenAtLogin(openAtLogin.IsOn));
            }
        };
        autoConnectCard = Cards.Card(L("Connect automatically at launch"), autoConnect);
        autoConnect.Toggled += (_, _) =>
        {
            if (!refreshing)
            {
                ui.Send(new Command.SetConnectionOption(new ConnectionOption.AutoConnectOnLaunch(autoConnect.IsOn)));
            }
        };
        startup.Children.Add(Cards.SectionHeader(L("Startup")));
        startup.Children.Add(openAtLoginCard);
        startup.Children.Add(openAtLoginError);
        startup.Children.Add(autoConnectCard);

        page.BodyPanel.Children.Add(status);
        page.BodyPanel.Children.Add(startup);
    }

    public void Refresh()
    {
        refreshing = true;
        status.Visible(ui.Connection is not null);
        if (ui.Connection is { } connection)
        {
            ShowStatus(connection, ui.Time);
        }
        startup.Visible(ui.Config is not null);
        if (ui.Config is { } config)
        {
            ShowStartup(config);
        }
        refreshing = false;
    }

    private void ShowStatus(ConnectionSection connection, TimeStyle time)
    {
        // The planned reconnect is left to the "Next proactive reconnect" card, as it would make the status long.
        currentText.Text = connection.StatusLine(ui.Schedule?.Outlook, time, includesPlannedReconnect: false);
        (currentMark.Glyph, currentMark.Foreground) = connection.Icon switch
        {
            ConnectionIcon.Connected => ("\uEC61", Cards.Brush("SystemFillColorSuccessBrush")),
            ConnectionIcon.Disconnected => ("\uE738", Cards.Brush("TextFillColorSecondaryBrush")),
            ConnectionIcon.InProgress => ("\uE895", Cards.Brush("TextFillColorSecondaryBrush")),
            ConnectionIcon.Error => ("\uE7BA", Cards.Brush("SystemFillColorCautionBrush")),
            _ => throw new InvalidOperationException($"unknown connection icon {connection.Icon}"),
        };
        current.Describe(connection.AccountAppliesNextConnection ? L("Account changes apply from the next connection.") : null);

        reason.Visible(connection.WaitReason is not null);
        reasonValue.Text = connection.WaitReason?.Summary(time) ?? "";

        var items = connection.Status is ConnectionStatus.SetupRequired setup ? setup.Items : [];
        if (!items.SequenceEqual(shownItems))
        {
            shownItems = items;
            setupItems.Children.Clear();
            foreach (var item in items)
            {
                var card = new SettingsCard { Header = item.ActionTitle(), IsClickEnabled = true };
                card.Click += (_, _) => ui.Settings.Show(item.Pane());
                setupItems.Children.Add(card);
            }
        }
        setupItems.Visible(items.Count > 0);

        shownFailure = (connection.Status as ConnectionStatus.Failed)?.Reason;
        failure.Show(shownFailure?.Guidance(time));
        failureAction.Content = shownFailure?.ActionTitle();

        var connected = connection.Status as ConnectionStatus.Connected;
        foreach (var card in new[] { connectedAt, tunnelIp, proactive })
        {
            card.Visible(connected is not null);
        }
        if (connected is not null)
        {
            connectedAtValue.Text = L("%@ (%@ ago)", time.ClockTime(connected.Since), time.Duration(time.Now - connected.Since));
            tunnelIpValue.Text = connected.TunnelIp ?? L("Unknown");
            proactiveValue.Text = connected.ProactiveReconnect switch
            {
                ProactiveReconnect.At at => $"{time.ClockTime(at.Time)} ({time.PlannedReconnect(at.Time)})",
                ProactiveReconnect.PausedBySchedule => L("Paused by the schedule"),
                ProactiveReconnect.Off => L("Off"),
                _ => throw new InvalidOperationException($"unknown proactive reconnect {connected.ProactiveReconnect}"),
            };
        }

        var next = connection.NextAttempt();
        nextAttempt.Visible(next is not null);
        nextAttemptValue.Text = next is { } nextAt ? time.ClockTime(nextAt) : "";

        // Always shown; the ones that don't apply now are dimmed, as in the menu.
        connect.IsEnabled = connection.Connect == ButtonState.Enabled;
        disconnect.IsEnabled = connection.Disconnect == ButtonState.Enabled;
        reconnectNow.IsEnabled = connection.ReconnectNow == ButtonState.Enabled;
    }

    private void ShowStartup(ConfigSection config)
    {
        var login = config.OpenAtLogin;
        openAtLogin.IsOn = login.Enabled;
        openAtLoginCard.IsEnabled = login.Available;
        openAtLoginCard.Describe(login.Available ? null : L("Only available for the app installed in Program Files."));
        // The description already gives the reason a copy outside Program Files can't set it.
        var error = login.Error is { } failed && (login.Available || failed is not RegistrationError.NotInstalledCopy) ? failed.LoginItemMessage() : null;
        openAtLoginError.Show(error);

        autoConnect.IsOn = config.Options.AutoConnectOnLaunch;
        autoConnectCard.Describe(ui.Schedule?.Enabled == true ? L("With the schedule on, this connects only during the scheduled hours.") : null);
    }
}
