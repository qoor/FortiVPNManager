using CommunityToolkit.WinUI.Controls;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

internal sealed class GeneralPane : IPane
{
    private readonly AppUi _ui;
    private readonly StackPanel _status = new() { Spacing = 4 };
    private readonly StackPanel _startup = new() { Spacing = 4 };
    private readonly StackPanel _import = new() { Spacing = 4 };
    private readonly StackPanel _importReport = new() { Spacing = 4 };
    private readonly TextBlock _importSource = Cards.Value();
    private readonly TextBlock _importServer = Cards.Value();
    private readonly InfoBar _importFailed = Cards.Notice(InfoBarSeverity.Error);
    private readonly SettingsCard _importClose = new() { ContentAlignment = ContentAlignment.Left };
    private readonly SettingsCard _current = Cards.Card(L("Current status"));
    private readonly FontIcon _currentMark;
    private readonly TextBlock _currentText;
    private readonly TextBlock _reasonValue = Cards.Value();
    private readonly SettingsCard _reason;
    private readonly StackPanel _setupItems = new() { Spacing = 4 };
    private readonly InfoBar _failure = Cards.Notice(InfoBarSeverity.Error);
    private readonly Button _failureAction = new();
    private readonly TextBlock _connectedAtValue = Cards.Value();
    private readonly TextBlock _tunnelIpValue = Cards.Value();
    private readonly TextBlock _proactiveValue = Cards.Value();
    private readonly TextBlock _nextAttemptValue = Cards.Value();
    private readonly SettingsCard _connectedAt;
    private readonly SettingsCard _tunnelIp;
    private readonly SettingsCard _proactive;
    private readonly SettingsCard _nextAttempt;
    private readonly Button _connect = new() { Content = L("Connect") };
    private readonly Button _disconnect = new() { Content = L("Disconnect") };
    private readonly Button _reconnectNow = new() { Content = L("Reconnect Now") };
    private readonly ToggleSwitch _openAtLogin = new();
    private readonly SettingsCard _openAtLoginCard;
    private readonly InfoBar _openAtLoginError = Cards.Notice(InfoBarSeverity.Error);
    private readonly ToggleSwitch _autoConnect = new();
    private readonly SettingsCard _autoConnectCard;
    private readonly SettingsCard _update = new() { Header = L("Software Update"), HeaderIcon = new FontIcon { Glyph = "\uE72C" }, IsClickEnabled = true };
    private readonly InfoBadge _updateDot = new() { Style = (Style)Application.Current.Resources["AttentionDotInfoBadgeStyle"] };
    private IReadOnlyList<SetupItem> _shownItems = [];
    private FailureReason? _shownFailure;
    /// <summary>The rows a search result goes to, by the setting's catalog key.</summary>
    private readonly Dictionary<string, FrameworkElement> _rows = [];
    private bool _refreshing;

    public GeneralPane(AppUi ui, PanePage page)
    {
        this._ui = ui;
        (Grid? currentLine, _currentMark, _currentText) = Cards.StatusLine();
        _current.Content = currentLine;
        _reason = Cards.Card(L("Reason"), _reasonValue);
        _connectedAt = Cards.Card(L("Connected at"), _connectedAtValue);
        _tunnelIp = Cards.Card(L("Tunnel IP"), _tunnelIpValue);
        _proactive = Cards.Card(L("Next proactive reconnect"), _proactiveValue);
        _nextAttempt = Cards.Card(L("Next attempt"), _nextAttemptValue);
        _failure.ActionButton = _failureAction;
        _failureAction.Click += (_, _) =>
        {
            if (_shownFailure is { } shown)
            {
                ui.Settings.Show(shown.Pane());
            }
        };
        _connect.Click += (_, _) => ui.Send(new Command.Connect());
        _disconnect.Click += (_, _) => ui.Send(new Command.Disconnect());
        _reconnectNow.Click += (_, _) => ui.Send(new Command.ReconnectNow());
        _status.Children.Add(Cards.SectionHeader(L("Status")));
        SettingsCard buttons = Cards.ActionCard(Cards.Buttons(_connect, _disconnect, _reconnectNow));
        _rows["Current status"] = _current;
        _rows["Connected at"] = _connectedAt;
        _rows["Tunnel IP"] = _tunnelIp;
        _rows["Next proactive reconnect"] = _proactive;
        _rows[SettingsSearch.ConnectionButtons] = buttons;
        foreach (UIElement element in new UIElement[] { _current, _reason, _setupItems, _failure, _connectedAt, _tunnelIp, _proactive, _nextAttempt, buttons })
        {
            _status.Children.Add(element);
        }

        _openAtLoginCard = Cards.Card(L("Open at login"), _openAtLogin);
        _openAtLogin.Toggled += (_, _) =>
        {
            if (!_refreshing)
            {
                ui.Send(new Command.SetOpenAtLogin(_openAtLogin.IsOn));
            }
        };
        _autoConnectCard = Cards.Card(L("Connect automatically at launch"), _autoConnect);
        _autoConnect.Toggled += (_, _) =>
        {
            if (!_refreshing)
            {
                ui.Send(new Command.SetConnectionOption(new ConnectionOption.AutoConnectOnLaunch(_autoConnect.IsOn)));
            }
        };
        _rows["Open at login"] = _openAtLoginCard;
        _rows["Connect automatically at launch"] = _autoConnectCard;
        _startup.Children.Add(Cards.SectionHeader(L("Startup")));
        _startup.Children.Add(_openAtLoginCard);
        _startup.Children.Add(_openAtLoginError);
        _startup.Children.Add(_autoConnectCard);

        _update.Content = _updateDot;
        _update.Click += (_, _) => ui.Settings.ShowUpdates();
        page.BodyPanel.Children.Add(_rows["Software Update"] = _update);
        page.BodyPanel.Children.Add(_status);
        page.BodyPanel.Children.Add(_startup);
        AddImportRows();
        page.BodyPanel.Children.Add(_import);
    }

    /// <summary>
    /// Importing FortiClient's settings, in the place macOS imports openfortivpn's. The report of the
    /// last import, the first launch's included, stays until closed.
    /// </summary>
    private void AddImportRows()
    {
        var importButton = new Button { Content = L("Import Settings") };
        importButton.Click += (_, _) => _ui.Send(new Command.ImportClientSettings());
        _import.Children.Add(Cards.SectionHeader(L("Import Settings")));
        SettingsCard source = Cards.Card(L("FortiClient settings"), importButton);
        source.Describe(L("Brings in the server address from FortiClient."));
        _import.Children.Add(_rows[SettingsSearch.ImportRow] = source);
        var close = new Button { Content = L("Close") };
        close.Click += (_, _) => _ui.Send(new Command.DismissImportReport());
        _importSource.TextWrapping = TextWrapping.Wrap;
        var report = new StackPanel { Spacing = 4 };
        report.Children.Add(_importSource);
        report.Children.Add(_importServer);
        _importReport.Children.Add(new SettingsCard { Content = report, ContentAlignment = ContentAlignment.Left });
        _import.Children.Add(_importReport);
        _import.Children.Add(_importFailed);
        // Closes the report, whether the import took the server or failed.
        _importClose.Content = close;
        _import.Children.Add(_importClose);
    }

    public FrameworkElement? OpenRow(string key) => _rows.GetValueOrDefault(key);

    public void Refresh()
    {
        _refreshing = true;
        _status.Visible(_ui.Connection is not null);
        if (_ui.Connection is { } connection)
        {
            ShowStatus(connection, _ui.Time);
        }
        _startup.Visible(_ui.Config is not null);
        _import.Visible(_ui.Config is not null);
        if (_ui.Config is { } config)
        {
            ShowStartup(config);
            ShowImport(config.Account.ImportReport);
        }
        _updateDot.Visible(_ui.AutoUpdate?.CardDot == true);
        _refreshing = false;
    }

    private void ShowImport(ClientSettingsImport? report)
    {
        ImportReport? imported = (report as ClientSettingsImport.Imported)?.Report;
        _importReport.Visible(imported is not null);
        if (imported is not null)
        {
            _importSource.Text = L("Imported from the FortiClient connection '%@'.", imported.Source);
            // Only the server came from FortiClient; the account's other values stay as they were.
            _importServer.Text = L("Server: %@:%@", imported.Host, imported.Port.ToString(System.Globalization.CultureInfo.InvariantCulture));
        }
        _importFailed.Show((report as ClientSettingsImport.Failed)?.Error.Message());
        _importClose.Visible(report is not null);
    }

    private void ShowStatus(ConnectionSection connection, TimeStyle time)
    {
        // The planned reconnect is left to the "Next proactive reconnect" card, as it would make the status long.
        _currentText.Text = connection.StatusLine(_ui.Schedule?.Outlook, time, includesPlannedReconnect: false);
        (_currentMark.Glyph, _currentMark.Foreground) = connection.Icon switch
        {
            ConnectionIcon.Connected => ("\uEC61", Cards.Brush("SystemFillColorSuccessBrush")),
            ConnectionIcon.Disconnected => ("\uE738", Cards.Brush("TextFillColorSecondaryBrush")),
            ConnectionIcon.InProgress => ("\uE895", Cards.Brush("TextFillColorSecondaryBrush")),
            ConnectionIcon.Error => ("\uE7BA", Cards.Brush("SystemFillColorCautionBrush")),
            _ => throw new InvalidOperationException($"unknown connection icon {connection.Icon}"),
        };
        _current.Describe(connection.AccountAppliesNextConnection ? L("Account changes apply from the next connection.") : null);

        _reason.Visible(connection.WaitReason is not null);
        _reasonValue.Text = connection.WaitReason?.Summary(time) ?? "";

        IReadOnlyList<SetupItem> items = connection.Status is ConnectionStatus.SetupRequired setup ? setup.Items : [];
        if (!items.SequenceEqual(_shownItems))
        {
            _shownItems = items;
            _setupItems.Children.Clear();
            foreach (SetupItem item in items)
            {
                var card = new SettingsCard { Header = item.ActionTitle(), IsClickEnabled = true };
                card.Click += (_, _) => _ui.Settings.Show(item.Pane());
                _setupItems.Children.Add(card);
            }
        }
        _setupItems.Visible(items.Count > 0);

        _shownFailure = (connection.Status as ConnectionStatus.Failed)?.Reason;
        _failure.Show(_shownFailure?.Guidance(time));
        _failureAction.Content = _shownFailure?.ActionTitle();

        var connected = connection.Status as ConnectionStatus.Connected;
        foreach (SettingsCard? card in new[] { _connectedAt, _tunnelIp, _proactive })
        {
            card.Visible(connected is not null);
        }
        if (connected is not null)
        {
            _connectedAtValue.Text = L("%@ (%@ ago)", time.ClockTime(connected.Since), TimeStyle.Duration(time.Now - connected.Since));
            _tunnelIpValue.Text = connected.TunnelIp ?? L("Unknown");
            _proactiveValue.Text = connected.ProactiveReconnect switch
            {
                ProactiveReconnect.At at => $"{time.ClockTime(at.Time)} ({time.PlannedReconnect(at.Time)})",
                ProactiveReconnect.PausedBySchedule => L("Paused by the schedule"),
                ProactiveReconnect.Off => L("Off"),
                _ => throw new InvalidOperationException($"unknown proactive reconnect {connected.ProactiveReconnect}"),
            };
        }

        DateTimeOffset? next = connection.NextAttempt();
        _nextAttempt.Visible(next is not null);
        _nextAttemptValue.Text = next is { } nextAt ? time.ClockTime(nextAt) : "";

        // Always shown; the ones that don't apply now are dimmed, as in the menu.
        _connect.IsEnabled = connection.Connect == ButtonState.Enabled;
        _disconnect.IsEnabled = connection.Disconnect == ButtonState.Enabled;
        _reconnectNow.IsEnabled = connection.ReconnectNow == ButtonState.Enabled;
    }

    private void ShowStartup(ConfigSection config)
    {
        OpenAtLogin login = config.OpenAtLogin;
        _openAtLogin.IsOn = login.Enabled;
        _openAtLoginCard.IsEnabled = login.Available;
        _openAtLoginCard.Describe(login.Available ? null : L("Only available for the app installed in Program Files."));
        // The description already gives the reason a copy outside Program Files can't set it.
        string? error = login.Error is { } failed && (login.Available || failed is not RegistrationError.NotInstalledCopy) ? failed.LoginItemMessage() : null;
        _openAtLoginError.Show(error);

        _autoConnect.IsOn = config.Options.AutoConnectOnLaunch;
        _autoConnectCard.Describe(_ui.Schedule?.Enabled == true ? L("With the schedule on, this connects only during the scheduled hours.") : null);
    }
}
