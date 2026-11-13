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
/// The notification-area icon, the menu bar item's place on Windows (docs/SPEC.md F-WN-01): the app's
/// icon, whose tooltip tells the connection's state; a click opens the menu the macOS menu bar item
/// has, drawn from the latest sections (components/ui/macos/MenuBar.swift), and a double-click Settings.
/// </summary>
/// <remarks>
/// The menu is the standard Windows menu of a notification-area icon, which H.NotifyIcon builds from
/// the items below each time it opens and sizes to their text. Its window-based menu takes the items
/// once and measures them before they are drawn, so it opened empty or cut off.
/// </remarks>
public sealed class TrayIcon : IDisposable
{
    private readonly AppUi ui;
    private readonly Action quit;
    private readonly TaskbarIcon icon;
    private readonly MenuFlyout menu = new();
    private readonly DispatcherQueue dispatcher = DispatcherQueue.GetForCurrentThread();
    private List<(string? Text, bool Enabled)> shownMenu = [];

    /// <param name="quit">Asks the runtime to quit, as the menu's Quit does.</param>
    /// <remarks>Made on the UI thread, where the menu's and the double-click's actions run.</remarks>
    public TrayIcon(AppUi ui, Action quit)
    {
        this.ui = ui;
        this.quit = quit;
        // The app's own icon, at the notification area's size for this display; the tooltip and the
        // menu tell the connection's state.
        var size = (int)Math.Round(16 * GetDpiForSystem() / 96.0);
        icon = new TaskbarIcon
        {
            Icon = new System.Drawing.Icon(Path.Combine(AppContext.BaseDirectory, "AppIcon.ico"), size, size),
            ContextMenuMode = ContextMenuMode.PopupMenu,
            MenuActivation = PopupActivationMode.LeftOrRightClick,
            ContextFlyout = menu,
            // A double-click opens Settings, as for other apps' icons there.
            DoubleClickCommand = new MenuCommand(dispatcher, () => ui.Settings.Show(null)),
        };
        Update();
        icon.ForceCreate(false);
        ui.Changed += Update;
    }

    public void Dispose()
    {
        ui.Changed -= Update;
        icon.Dispose();
    }

    private void Update()
    {
        icon.ToolTipText = "FortiVPNManager: " + (ui.Connection?.Icon ?? ConnectionIcon.Disconnected).AccessibilityText();
        ShowMenu(Entries());
    }

    /// <summary>The items again, only when what they say changed: the clock's ticks mostly change nothing here.</summary>
    private void ShowMenu(List<Entry> entries)
    {
        var content = entries.Select(entry => (entry.Text, entry.Enabled)).ToList();
        if (content.SequenceEqual(shownMenu))
        {
            return;
        }
        shownMenu = content;
        menu.Items.Clear();
        foreach (var entry in entries)
        {
            // The standard menu takes "&" as the mark of a shortcut key; "&&" shows the "&" itself.
            menu.Items.Add(entry.Text is null
                ? new MenuFlyoutSeparator()
                : new MenuFlyoutItem { Text = entry.Text.Replace("&", "&&", StringComparison.Ordinal), IsEnabled = entry.Enabled, Command = entry.Run is { } run ? new MenuCommand(dispatcher, run) : null });
        }
    }

    /// <summary>The menu's items: the status lines, the setup or failure actions, the connection's buttons, then the app's.</summary>
    private List<Entry> Entries()
    {
        var entries = new List<Entry>();
        if (ui.Connection is { } connection)
        {
            var time = ui.Time;
            var outlook = ui.Schedule?.Outlook;
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
                    foreach (var item in setup.Items)
                    {
                        entries.Add(new Entry(item.ActionTitle(), true, () => ui.Settings.Show(item.Pane())));
                    }
                    break;
                case ConnectionStatus.Failed failed:
                    entries.Add(Line(failed.Reason.Guidance(time)));
                    entries.Add(new Entry(failed.Reason.ActionTitle(), true, () => ui.Settings.Show(failed.Reason.Pane())));
                    break;
            }
            // Always listed; the ones that don't apply now are dimmed.
            entries.Add(Separator);
            entries.Add(new Entry(L("Connect"), connection.Connect == ButtonState.Enabled, () => ui.Send(new Command.Connect())));
            entries.Add(new Entry(L("Disconnect"), connection.Disconnect == ButtonState.Enabled, () => ui.Send(new Command.Disconnect())));
            entries.Add(new Entry(L("Reconnect Now"), connection.ReconnectNow == ButtonState.Enabled, () => ui.Send(new Command.ReconnectNow())));
        }
        entries.Add(Separator);
        entries.Add(new Entry(L("Show Log"), true, () => ui.Settings.Show(SettingsPane.Log)));
        entries.Add(new Entry(L("Settings…"), true, () => ui.Settings.Show(null)));
        entries.Add(Separator);
        entries.Add(new Entry(L("Quit FortiVPNManager"), true, quit));
        return entries;
    }

    private static readonly Entry Separator = new(null, false, null);

    /// <summary>A line of text, dimmed as the macOS menu's text lines are.</summary>
    private static Entry Line(string text) => new(text, false, null);

    /// <summary>A menu item: its text (none for a separator), whether it can be chosen, and what choosing it does.</summary>
    private sealed record Entry(string? Text, bool Enabled, Action? Run);

    /// <summary>The standard menu runs an item through its command; the action runs on the UI thread.</summary>
    private sealed class MenuCommand(DispatcherQueue dispatcher, Action action) : ICommand
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
