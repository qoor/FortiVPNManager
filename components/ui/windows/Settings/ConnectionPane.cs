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
    private readonly AppUi ui;
    private readonly StackPanel body = new() { Spacing = 4 };
    private readonly ToggleSwitch scheduleEnabled = new();
    private readonly List<SettingsCard> scheduleRows = [];
    private readonly Dictionary<Weekday, ToggleButton> days = [];
    private readonly TimePicker start = new();
    private readonly TimePicker end = new();
    private readonly InfoBar hoursError = Cards.Notice(InfoBarSeverity.Error);
    private readonly MinutesBox awayMinutes;
    private readonly TextBlock outlookValue = Cards.Value();
    private readonly SettingsCard outlook;
    private readonly MinutesBox sessionMax;
    private readonly ToggleSwitch proactive = new();
    private readonly MinutesBox margin;
    private readonly SettingsCard marginCard;
    private readonly ToggleSwitch afterSleep = new();
    private readonly MinutesBox minSleep;
    private readonly SettingsCard minSleepCard;
    private readonly InfoBar optionsError = Cards.Notice(InfoBarSeverity.Error);
    private ScheduleSection? schedule;
    private bool refreshing;

    public ConnectionPane(AppUi ui, PanePage page)
    {
        this.ui = ui;
        awayMinutes = new MinutesBox(this, minutes => new ConnectionOption.AwayMinutes(minutes));
        sessionMax = new MinutesBox(this, minutes => new ConnectionOption.SessionMaxMinutes(minutes));
        margin = new MinutesBox(this, minutes => new ConnectionOption.ProactiveMarginMinutes(minutes));
        minSleep = new MinutesBox(this, minutes => new ConnectionOption.MinSleepMinutes(minutes));
        outlook = Cards.Card(L("Next"), outlookValue);
        marginCard = Cards.Card(L("Margin"), margin.Row);
        minSleepCard = Cards.Card(L("Minimum sleep time"), minSleep.Row);
        AddScheduleRows();
        AddReconnectRows();
        page.BodyPanel.Children.Add(body);
    }

    private void AddScheduleRows()
    {
        body.Children.Add(Cards.SectionHeader(L("Schedule"), first: true));
        var expander = new SettingsExpander { Header = L("Connect on a schedule"), Content = Switch(scheduleEnabled, on => new ConnectionOption.ScheduleEnabled(on)), IsExpanded = true };
        // Seven equal buttons, 40 px each beside the header and narrower when the card is narrow.
        var dayRow = new Grid { ColumnSpacing = 4 };
        foreach (var day in Enum.GetValues<Weekday>())
        {
            var button = new ToggleButton { Content = day.ShortName(), HorizontalAlignment = HorizontalAlignment.Stretch, Padding = new Thickness(0, 5, 0, 5) };
            Grid.SetColumn(button, dayRow.ColumnDefinitions.Count);
            dayRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            // Click comes only from the person, unlike Checked, which the refresh would also raise.
            button.Click += (_, _) => Send(new ConnectionOption.ScheduleDay(day, button.IsChecked == true));
            days[day] = button;
            dayRow.Children.Add(button);
        }
        scheduleRows.Add(Cards.Card(L("Days"), dayRow).Fit(dayRow, (7 * 40) + (6 * 4)));
        start.SelectedTimeChanged += (_, _) => HoursPicked();
        end.SelectedTimeChanged += (_, _) => HoursPicked();
        // The picker and the button inside it keep 242 px, which would run past a narrow card.
        foreach (var picker in new[] { start, end })
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
        scheduleRows.Add(Cards.Card(L("Start time"), start).Fit(start, 242));
        scheduleRows.Add(Cards.Card(L("End time"), end).Fit(end, 242));
        var away = Cards.Card(L("Stop reconnecting after being away"), awayMinutes.Row);
        ToolTipService.SetToolTip(away, L("After the end time, reconnects wait while you're away (screen locked or off, PC asleep) and stop after this long."));
        scheduleRows.Add(away);
        scheduleRows.Add(outlook);
        foreach (var row in scheduleRows)
        {
            expander.Items.Add(row);
        }
        body.Children.Add(expander);
        body.Children.Add(hoursError);
        body.Children.Add(Cards.Footer(L("Connects at the start time on the selected days. The end time never disconnects.")));
    }

    private void AddReconnectRows()
    {
        body.Children.Add(Cards.SectionHeader(L("Reconnect")));
        body.Children.Add(Cards.Card(L("Maximum session length"), sessionMax.Row));
        var proactiveRow = new SettingsExpander { Header = L("Proactive reconnect"), Content = Switch(proactive, on => new ConnectionOption.ProactiveReconnect(on)), IsExpanded = true };
        proactiveRow.Items.Add(marginCard);
        body.Children.Add(proactiveRow);
        var sleepRow = new SettingsExpander { Header = L("Reconnect after sleep"), Content = Switch(afterSleep, on => new ConnectionOption.ReconnectAfterSleep(on)), IsExpanded = true };
        ToolTipService.SetToolTip(sleepRow, L("Starts a new connection when the PC wakes from sleeping at least the minimum sleep time."));
        sleepRow.Items.Add(minSleepCard);
        body.Children.Add(sleepRow);
        body.Children.Add(optionsError);
        body.Children.Add(Cards.Footer(L("Connects again before the server ends the session (maximum session length − margin).")));
    }

    public void FinishTyping()
    {
        foreach (var minutes in new[] { awayMinutes, sessionMax, margin, minSleep })
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
        if (!refreshing)
        {
            ui.Send(new Command.SetConnectionOption(option));
        }
    }

    /// <summary>
    /// Every change of either picker is sent; the runtime saves them only while the end is later than
    /// the start, so the pickers keep what was picked meanwhile. Loading the saved times is not sent
    /// again unless an earlier choice is still unsaved.
    /// </summary>
    private void HoursPicked()
    {
        if (refreshing || schedule is null || start.SelectedTime is not { } startTime || end.SelectedTime is not { } endTime)
        {
            return;
        }
        var newStart = new TimeOfDay((byte)startTime.Hours, (byte)startTime.Minutes);
        var newEnd = new TimeOfDay((byte)endTime.Hours, (byte)endTime.Minutes);
        if (newStart != schedule.Start || newEnd != schedule.End || schedule.HoursError is not null)
        {
            ui.Send(new Command.SetScheduleHours(newStart, newEnd));
        }
    }

    public void Refresh()
    {
        body.Visible(ui.Schedule is not null && ui.Config is not null);
        if (ui.Schedule is not { } current || ui.Config is not { } config)
        {
            return;
        }
        refreshing = true;
        var hoursChanged = schedule is null || schedule.Start != current.Start || schedule.End != current.End;
        schedule = current;
        scheduleEnabled.IsOn = current.Enabled;
        foreach (var (day, button) in days)
        {
            button.IsChecked = current.Days.Contains(day);
        }
        // A pick that wasn't saved stays in its picker until the saved times change.
        if (hoursChanged)
        {
            start.SelectedTime = new TimeSpan(current.Start.Hour, current.Start.Minute, 0);
            end.SelectedTime = new TimeSpan(current.End.Hour, current.End.Minute, 0);
        }
        hoursError.Show(current.HoursError?.Message());
        awayMinutes.Show(current.AwayMinutes);
        outlook.Visible(current.Enabled && current.Outlook is not null);
        outlookValue.Text = current.Outlook?.Summary(ui.Time) ?? "";
        foreach (var row in scheduleRows)
        {
            row.IsEnabled = current.Enabled;
        }

        var options = config.Options;
        sessionMax.Show(options.SessionMaxMinutes);
        proactive.IsOn = options.ProactiveReconnect;
        margin.Show(options.MarginMinutes);
        marginCard.IsEnabled = options.ProactiveReconnect;
        afterSleep.IsOn = options.ReconnectAfterSleep;
        minSleep.Show(options.MinSleepMinutes);
        minSleepCard.IsEnabled = options.ReconnectAfterSleep;
        optionsError.Show(config.OptionsError?.Message());
        refreshing = false;
    }

    /// <summary>
    /// A minutes setting: a number box that commits on Enter or when it loses focus (never per
    /// keystroke, which would save 18 on the way to 180), with spin buttons within the range the
    /// settings file accepts. Page Up and Page Down step by 10, as Windows number boxes do.
    /// </summary>
    private sealed class MinutesBox
    {
        // Wide enough for the number beside the spin buttons (72) and the clear button (30) shown while typing.
        private readonly NumberBox box = new()
        {
            Width = 160,
            SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline,
            SmallChange = 1,
            LargeChange = 10,
        };
        private readonly ConnectionPane pane;
        private readonly Func<long, ConnectionOption> change;
        private MinutesSetting? setting;

        public MinutesBox(ConnectionPane pane, Func<long, ConnectionOption> change)
        {
            this.pane = pane;
            this.change = change;
            ToolTipService.SetToolTip(box, L("Page Up and Page Down change it by 10 minutes"));
            box.ValueChanged += (_, args) =>
            {
                if (pane.refreshing || setting is null)
                {
                    return;
                }
                if (double.IsNaN(args.NewValue))
                {
                    box.Value = setting.Value;
                    return;
                }
                var minutes = (long)Math.Round(args.NewValue);
                if (minutes != setting.Value)
                {
                    pane.Send(change(minutes));
                }
            };
            Row = Cards.Row(box, new TextBlock { Text = L("min"), VerticalAlignment = VerticalAlignment.Center });
        }

        public StackPanel Row { get; }

        /// <summary>
        /// The saved setting, put in the box only when it changed: a number the runtime turned down stays
        /// in the box beside the reason, as on macOS, until the setting changes.
        /// </summary>
        public void Show(MinutesSetting current)
        {
            if (current == setting)
            {
                return;
            }
            setting = current;
            box.Minimum = current.Minimum;
            box.Maximum = current.Maximum;
            box.Value = current.Value;
        }

        /// <summary>Sends a number still being typed, kept within the range as the box keeps it.</summary>
        public void Commit()
        {
            if (setting is not null && long.TryParse(box.Text.Trim(), out var typed))
            {
                var minutes = Math.Clamp(typed, setting.Minimum, setting.Maximum);
                if (minutes != setting.Value)
                {
                    pane.Send(change(minutes));
                }
            }
        }
    }
}
