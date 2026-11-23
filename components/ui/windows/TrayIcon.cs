using System.Drawing;
using System.Runtime.InteropServices;
using System.Windows.Input;
using FortiVPNManager.Ui.Text;
using H.NotifyIcon;
using H.NotifyIcon.Core;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Controls;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui;

/// <summary>
/// The notification-area icon, the menu bar item's place on Windows: a click opens the menu the macOS
/// menu bar item has, drawn from the latest sections (components/ui/macos/MenuBar.swift), and a
/// double-click Settings.
/// </summary>
/// <remarks>
/// The menu is the standard Windows menu of a notification-area icon, which H.NotifyIcon builds from
/// the items below each time it opens and sizes to their text. Its window-based menu takes the items
/// once and measures them before they are drawn, so it opened empty or cut off.
/// </remarks>
public sealed partial class TrayIcon : IDisposable
{
    private readonly AppUi _ui;
    private readonly Action _quit;
    private readonly TaskbarIcon _icon;
    private readonly Dictionary<ConnectionIcon, System.Drawing.Icon> _stateIcons;
    private readonly MenuFlyout _menu = new();
    private readonly DispatcherQueue _dispatcher = DispatcherQueue.GetForCurrentThread();
    private ConnectionIcon? _shownIcon;
    private List<(string? Text, bool Enabled)> _shownMenu = [];

    /// <param name="quit">Asks the runtime to quit, as the menu's Quit does.</param>
    /// <remarks>Made on the UI thread, where the menu's and the double-click's actions run.</remarks>
    public TrayIcon(AppUi ui, Action quit)
    {
        this._ui = ui;
        this._quit = quit;
        int size = (int)Math.Round(16 * GetDpiForSystem() / 96.0);
        _stateIcons = Enum.GetValues<ConnectionIcon>().ToDictionary(state => state, state => new System.Drawing.Icon(Path.Combine(AppContext.BaseDirectory, IconFile(state)), size, size));
        _icon = new TaskbarIcon
        {
            ContextMenuMode = ContextMenuMode.PopupMenu,
            MenuActivation = PopupActivationMode.LeftOrRightClick,
            ContextFlyout = _menu,
            // A double-click opens Settings, as for other apps' icons there.
            DoubleClickCommand = new MenuCommand(_dispatcher, () => ui.Settings.Show(null)),
        };
        Update();
        _icon.ForceCreate(false);
        ui.Changed += Update;
    }

    public void Dispose()
    {
        _ui.Changed -= Update;
        _icon.Dispose();
        foreach (Icon stateIcon in _stateIcons.Values)
        {
            stateIcon.Dispose();
        }
    }

    private void Update()
    {
        ConnectionIcon state = _ui.Connection?.Icon ?? ConnectionIcon.Disconnected;
        _icon.ToolTipText = "FortiVPNManager: " + state.AccessibilityText();
        if (state != _shownIcon)
        {
            _shownIcon = state;
            _ = _icon.UpdateIcon(_stateIcons[state]);
        }
        ShowMenu(Entries());
    }

    /// <summary>The items again, only when what they say changed: the clock's ticks mostly change nothing here.</summary>
    private void ShowMenu(List<Entry> entries)
    {
        var content = entries.Select(entry => (entry.Text, entry.Enabled)).ToList();
        if (content.SequenceEqual(_shownMenu))
        {
            return;
        }
        _shownMenu = content;
        _menu.Items.Clear();
        foreach (Entry entry in entries)
        {
            // The standard menu takes "&" as the mark of a shortcut key; "&&" shows the "&" itself.
            _menu.Items.Add(entry.Text is null
                ? new MenuFlyoutSeparator()
                : new MenuFlyoutItem { Text = entry.Text.Replace("&", "&&", StringComparison.Ordinal), IsEnabled = entry.Enabled, Command = entry.Run is { } run ? new MenuCommand(_dispatcher, run) : null });
        }
    }

    /// <summary>The menu's items: the status lines, the setup or failure actions, the connection's buttons, then the app's.</summary>
    private List<Entry> Entries()
    {
        var entries = new List<Entry>();
        if (_ui.Connection is { } connection)
        {
            TimeStyle time = _ui.Time;
            ScheduleOutlook? outlook = _ui.Schedule?.Outlook;
            entries.Add(Line(connection.StatusLine(outlook, time)));
            if (connection.Status is ConnectionStatus.Connected connected)
            {
                entries.Add(Line(L("Tunnel IP %@ · since %@", connected.TunnelIp ?? L("Unknown"), time.ClockTime(connected.Since))));
            }
            if (connection.WaitReason is { } reason)
            {
                entries.Add(Line(reason.Summary(time)));
            }
            if (outlook is not null)
            {
                entries.Add(Line(L("Schedule: %@", outlook.Summary(time))));
            }
            switch (connection.Status)
            {
                case ConnectionStatus.SetupRequired setup:
                    foreach (SetupItem item in setup.Items)
                    {
                        entries.Add(new Entry(item.ActionTitle(), true, () => _ui.Settings.Show(item.Pane())));
                    }
                    break;
                case ConnectionStatus.Failed failed:
                    entries.Add(Line(failed.Reason.Guidance(time)));
                    entries.Add(new Entry(failed.Reason.ActionTitle(), true, () => _ui.Settings.Show(failed.Reason.Pane())));
                    break;
            }
            // Always listed; the ones that don't apply now are dimmed.
            entries.Add(s_separator);
            entries.Add(new Entry(L("Connect"), connection.Connect == ButtonState.Enabled, () => _ui.Send(new Command.Connect())));
            entries.Add(new Entry(L("Disconnect"), connection.Disconnect == ButtonState.Enabled, () => _ui.Send(new Command.Disconnect())));
            entries.Add(new Entry(L("Reconnect Now"), connection.ReconnectNow == ButtonState.Enabled, () => _ui.Send(new Command.ReconnectNow())));
        }
        entries.Add(s_separator);
        entries.Add(new Entry(L("Show Log"), true, () => _ui.Settings.Show(SettingsPane.Log)));
        entries.Add(new Entry(L("Settings…"), true, () => _ui.Settings.Show(null)));
        entries.Add(s_separator);
        entries.Add(new Entry(L("Quit FortiVPNManager"), true, _quit));
        return entries;
    }

    private static readonly Entry s_separator = new(null, false, null);

    /// <summary>A line of text, dimmed as the macOS menu's text lines are.</summary>
    private static Entry Line(string text) => new(text, false, null);

    /// <summary>A menu item: its text (none for a separator), whether it can be chosen, and what choosing it does.</summary>
    private sealed record Entry(string? Text, bool Enabled, Action? Run);

    /// <summary>The standard menu runs an item through its command; the action runs on the UI thread.</summary>
    /// <remarks>
    /// The menu takes the command through WinRT, whose code for it a generator adds when the app is
    /// built for Native AOT; that takes it and the class around it to be partial.
    /// </remarks>
    private sealed partial class MenuCommand(DispatcherQueue dispatcher, Action action) : ICommand
    {
        public event EventHandler? CanExecuteChanged
        {
            add { }
            remove { }
        }

        public bool CanExecute(object? parameter) => true;

        public void Execute(object? parameter) => dispatcher.TryEnqueue(() => action());
    }

    [DllImport("user32.dll")]
    private static extern uint GetDpiForSystem();
}
