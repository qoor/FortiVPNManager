using CommunityToolkit.WinUI.Controls;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Connection pane (components/ui/macos/ConnectionPane.swift): the schedule and the reconnect
/// options, saved as they change. A switch that has settings of its own leads an expander with them,
/// as in Windows Settings.
/// </summary>
internal sealed class ConnectionPane : IPane
{
    private readonly AppUi _ui;
    private readonly StackPanel _body = new() { Spacing = 4 };
    private readonly ToggleSwitch _scheduleEnabled = new();
    private readonly List<SettingsCard> _scheduleRows = [];
    private readonly Dictionary<Weekday, ToggleButton> _days = [];
    private readonly TimePicker _start = new();
    private readonly TimePicker _end = new();
    private readonly InfoBar _hoursError = Cards.Notice(InfoBarSeverity.Error);
    private readonly MinutesBox _awayMinutes;
    private readonly TextBlock _outlookValue = Cards.Value();
    private readonly SettingsCard _outlook;
    private readonly MinutesBox _sessionMax;
    private readonly ToggleSwitch _proactive = new();
    private readonly MinutesBox _margin;
    private readonly SettingsCard _marginCard;
    private readonly ToggleSwitch _afterSleep = new();
    private readonly MinutesBox _minSleep;
    private readonly SettingsCard _minSleepCard;
    private readonly InfoBar _optionsError = Cards.Notice(InfoBarSeverity.Error);
    /// <summary>The rows a search result goes to, by the setting's catalog key.</summary>
    private readonly Dictionary<string, FrameworkElement> _rows = [];
    /// <summary>The expander each row in one is in, which the person may have collapsed.</summary>
    private readonly Dictionary<FrameworkElement, SettingsExpander> _expanders = [];
    private ScheduleSection? _schedule;
    private bool _refreshing;

    public ConnectionPane(AppUi ui, PanePage page)
    {
        this._ui = ui;
        _awayMinutes = new MinutesBox(this, minutes => new ConnectionOption.AwayMinutes(minutes));
        _sessionMax = new MinutesBox(this, minutes => new ConnectionOption.SessionMaxMinutes(minutes));
        _margin = new MinutesBox(this, minutes => new ConnectionOption.ProactiveMarginMinutes(minutes));
        _minSleep = new MinutesBox(this, minutes => new ConnectionOption.MinSleepMinutes(minutes));
        _outlook = Cards.Card(L("Next"), _outlookValue);
        _marginCard = Cards.Card(L("Margin"), _margin.Row);
        _minSleepCard = Cards.Card(L("Minimum sleep time"), _minSleep.Row);
        AddScheduleRows();
        AddReconnectRows();
        page.BodyPanel.Children.Add(_body);
    }

    private void AddScheduleRows()
    {
        _body.Children.Add(Cards.SectionHeader(L("Schedule"), first: true));
        var expander = new SettingsExpander { Header = L("Connect on a schedule"), Content = Switch(_scheduleEnabled, on => new ConnectionOption.ScheduleEnabled(on)), IsExpanded = true };
        // Seven equal buttons, 40 px each beside the header and narrower when the card is narrow.
        var dayRow = new Grid { ColumnSpacing = 4 };
        foreach (Weekday day in Enum.GetValues<Weekday>())
        {
            var button = new ToggleButton { Content = day.ShortName(), HorizontalAlignment = HorizontalAlignment.Stretch, Padding = new Thickness(0, 5, 0, 5) };
            Grid.SetColumn(button, dayRow.ColumnDefinitions.Count);
            dayRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            // Click comes only from the person, unlike Checked, which the refresh would also raise.
            button.Click += (_, _) => Send(new ConnectionOption.ScheduleDay(day, button.IsChecked == true));
            _days[day] = button;
            dayRow.Children.Add(button);
        }
        _scheduleRows.Add((SettingsCard)(_rows["Days"] = Cards.Card(L("Days"), dayRow).Fit(dayRow, (7 * 40) + (6 * 4))));
        _start.SelectedTimeChanged += (_, _) => HoursPicked();
        _end.SelectedTimeChanged += (_, _) => HoursPicked();
        // The picker and the button inside it keep 242 px, which would run past a narrow card.
        foreach (TimePicker? picker in new[] { _start, _end })
        {
            picker.MinWidth = 0;
            picker.Loaded += (_, _) =>
            {
                if (Cards.FindDescendant(picker, "FlyoutButton") is FrameworkElement button)
                {
                    button.MinWidth = 0;
                }
            };
        }
        _scheduleRows.Add((SettingsCard)(_rows["Start time"] = Cards.Card(L("Start time"), _start).Fit(_start, 242)));
        _scheduleRows.Add((SettingsCard)(_rows["End time"] = Cards.Card(L("End time"), _end).Fit(_end, 242)));
        SettingsCard away = Cards.Card(L("Stop reconnecting after being away"), _awayMinutes.Row);
        ToolTipService.SetToolTip(away, L("After the end time, reconnects wait while you're away (screen locked or off, PC asleep) and stop after this long."));
        _scheduleRows.Add(away);
        _rows["Stop reconnecting after being away"] = away;
        _scheduleRows.Add(_outlook);
        foreach (SettingsCard row in _scheduleRows)
        {
            expander.Items.Add(row);
            _expanders[row] = expander;
        }
        _body.Children.Add(_rows["Connect on a schedule"] = expander);
        _body.Children.Add(_hoursError);
        _body.Children.Add(Cards.Footer(L("Connects at the start time on the selected days. The end time never disconnects.")));
    }

    private void AddReconnectRows()
    {
        _body.Children.Add(Cards.SectionHeader(L("Reconnect")));
        _body.Children.Add(_rows["Maximum session length"] = Cards.Card(L("Maximum session length"), _sessionMax.Row));
        var proactiveRow = new SettingsExpander { Header = L("Proactive reconnect"), Content = Switch(_proactive, on => new ConnectionOption.ProactiveReconnect(on)), IsExpanded = true };
        proactiveRow.Items.Add(_marginCard);
        _body.Children.Add(_rows["Proactive reconnect"] = proactiveRow);
        _rows["Margin"] = _marginCard;
        _expanders[_marginCard] = proactiveRow;
        var sleepRow = new SettingsExpander { Header = L("Reconnect after sleep"), Content = Switch(_afterSleep, on => new ConnectionOption.ReconnectAfterSleep(on)), IsExpanded = true };
        ToolTipService.SetToolTip(sleepRow, L("Starts a new connection when the PC wakes from sleeping at least the minimum sleep time."));
        sleepRow.Items.Add(_minSleepCard);
        _body.Children.Add(_rows["Reconnect after sleep"] = sleepRow);
        _rows["Minimum sleep time"] = _minSleepCard;
        _expanders[_minSleepCard] = sleepRow;
        _body.Children.Add(_optionsError);
        _body.Children.Add(Cards.Footer(L("Connects again before the server ends the session (maximum session length − margin).")));
    }

    public void FinishTyping()
    {
        foreach (MinutesBox? minutes in new[] { _awayMinutes, _sessionMax, _margin, _minSleep })
        {
            minutes.Commit();
        }
    }

    /// <summary>A switch of the connection or the schedule, saved when flipped.</summary>
    private ToggleSwitch Switch(ToggleSwitch toggle, Func<bool, ConnectionOption> change)
    {
        toggle.Toggled += (_, _) => Send(change(toggle.IsOn));
        return toggle;
    }

    private void Send(ConnectionOption option)
    {
        if (!_refreshing)
        {
            _ui.Send(new Command.SetConnectionOption(option));
        }
    }

    /// <summary>
    /// Every change of either picker is sent; the runtime saves them only while the end is later than
    /// the start, so the pickers keep what was picked meanwhile. Loading the saved times is not sent
    /// again unless an earlier choice is still unsaved.
    /// </summary>
    private void HoursPicked()
    {
        if (_refreshing || _schedule is null || _start.SelectedTime is not { } startTime || _end.SelectedTime is not { } endTime)
        {
            return;
        }
        var newStart = new TimeOfDay((byte)startTime.Hours, (byte)startTime.Minutes);
        var newEnd = new TimeOfDay((byte)endTime.Hours, (byte)endTime.Minutes);
        if (newStart != _schedule.Start || newEnd != _schedule.End || _schedule.HoursError is not null)
        {
            _ui.Send(new Command.SetScheduleHours(newStart, newEnd));
        }
    }

    public FrameworkElement? OpenRow(string key)
    {
        FrameworkElement? row = _rows.GetValueOrDefault(key);
        if (row is not null && _expanders.TryGetValue(row, out SettingsExpander? expander))
        {
            expander.IsExpanded = true;
        }
        return row;
    }

    public void Refresh()
    {
        _body.Visible(_ui.Schedule is not null && _ui.Config is not null);
        if (_ui.Schedule is not { } current || _ui.Config is not { } config)
        {
            return;
        }
        _refreshing = true;
        bool hoursChanged = _schedule is null || _schedule.Start != current.Start || _schedule.End != current.End;
        _schedule = current;
        _scheduleEnabled.IsOn = current.Enabled;
        foreach ((Weekday day, ToggleButton? button) in _days)
        {
            button.IsChecked = current.Days.Contains(day);
        }
        // A pick that wasn't saved stays in its picker until the saved times change.
        if (hoursChanged)
        {
            _start.SelectedTime = new TimeSpan(current.Start.Hour, current.Start.Minute, 0);
            _end.SelectedTime = new TimeSpan(current.End.Hour, current.End.Minute, 0);
        }
        _hoursError.Show(current.HoursError?.Message());
        _awayMinutes.Show(current.AwayMinutes);
        _outlook.Visible(current.Enabled && current.Outlook is not null);
        _outlookValue.Text = current.Outlook?.Summary(_ui.Time) ?? "";
        foreach (SettingsCard row in _scheduleRows)
        {
            row.IsEnabled = current.Enabled;
        }

        ConnectionOptions options = config.Options;
        _sessionMax.Show(options.SessionMaxMinutes);
        _proactive.IsOn = options.ProactiveReconnect;
        _margin.Show(options.MarginMinutes);
        _marginCard.IsEnabled = options.ProactiveReconnect;
        _afterSleep.IsOn = options.ReconnectAfterSleep;
        _minSleep.Show(options.MinSleepMinutes);
        _minSleepCard.IsEnabled = options.ReconnectAfterSleep;
        _optionsError.Show(config.OptionsError?.Message());
        _refreshing = false;
    }

    /// <summary>
    /// A minutes setting: a number box that commits on Enter or when it loses focus (never per
    /// keystroke, which would save 18 on the way to 180), with spin buttons within the range the
    /// settings file accepts. Page Up and Page Down step by 10, as Windows number boxes do.
    /// </summary>
    private sealed class MinutesBox
    {
        // Wide enough for the number beside the spin buttons (72) and the clear button (30) shown while typing.
        private readonly NumberBox _box = new()
        {
            Width = 160,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline,
            SmallChange = 1,
            LargeChange = 10,
        };
        private readonly ConnectionPane _pane;
        private readonly Func<long, ConnectionOption> _change;
        private MinutesSetting? _setting;

        public MinutesBox(ConnectionPane pane, Func<long, ConnectionOption> change)
        {
            this._pane = pane;
            this._change = change;
            ToolTipService.SetToolTip(_box, L("Page Up and Page Down change it by 10 minutes"));
            _box.ValueChanged += (_, args) =>
            {
                if (pane._refreshing || _setting is null)
                {
                    return;
                }
                if (double.IsNaN(args.NewValue))
                {
                    _box.Value = _setting.Value;
                    return;
                }
                long minutes = (long)Math.Round(args.NewValue);
                if (minutes != _setting.Value)
                {
                    pane.Send(change(minutes));
                }
            };
            Row = Cards.Row(_box, new TextBlock { Text = L("min"), VerticalAlignment = VerticalAlignment.Center });
        }

        public StackPanel Row { get; }

        /// <summary>
        /// The saved setting, put in the box only when it changed: a number the runtime turned down stays
        /// in the box beside the reason, as on macOS, until the setting changes.
        /// </summary>
        public void Show(MinutesSetting current)
        {
            if (current == _setting)
            {
                return;
            }
            _setting = current;
            _box.Minimum = current.Minimum;
            _box.Maximum = current.Maximum;
            _box.Value = current.Value;
        }

        /// <summary>Sends a number still being typed, kept within the range as the box keeps it.</summary>
        public void Commit()
        {
            if (_setting is not null && long.TryParse(_box.Text.Trim(), out long typed))
            {
                long minutes = Math.Clamp(typed, _setting.Minimum, _setting.Maximum);
                if (minutes != _setting.Value)
                {
                    _pane.Send(_change(minutes));
                }
            }
        }
    }
}
