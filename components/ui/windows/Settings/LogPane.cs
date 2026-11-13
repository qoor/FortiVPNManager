using Microsoft.UI.Input;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Windows.System;
using Windows.UI;
using Windows.UI.Core;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Log pane (components/ui/macos/LogPane.swift): the VPN client's raw output and the explanation
/// lines of the app and the Helper, in a box that fills the pane. Its commands sit at the top: find
/// (Ctrl+F), clear the view (Ctrl+L), copy everything, and the log file in File Explorer.
/// </summary>
internal sealed class LogPane : IPane, IDisposable
{
    /// <summary>
    /// Lines trimmed from the app's log leave the view in chunks: removing text at the top lays out
    /// everything below it again, while appending lays out only the new lines.
    /// </summary>
    private const int TrimChunk = 500;

    private readonly AppUi ui;
    private readonly RichEditBox view = new()
    {
        IsReadOnly = true,
        TextWrapping = TextWrapping.Wrap,
        FontFamily = new FontFamily("Cascadia Mono, Consolas"),
        FontSize = 12,
        IsSpellCheckEnabled = false,
        IsTextPredictionEnabled = false,
        BorderThickness = new Thickness(0),
        Padding = new Thickness(8, 6, 8, 6),
    };
    private readonly CheckBox autoScroll = new() { Content = L("Auto-scroll") };
    private readonly StackPanel findBar = new() { Orientation = Orientation.Horizontal, Spacing = 4, Visibility = Visibility.Collapsed, HorizontalAlignment = HorizontalAlignment.Right };
    private readonly TextBox findBox = new() { PlaceholderText = L("Find in log"), Width = 240 };
    private readonly TextBlock noMatch = new() { Text = L("No results"), VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 0, 0), Visibility = Visibility.Collapsed };
    private readonly AppBarButton showFile;

    /// <summary>The entries shown, oldest first, with their length in the view (each ends with a line break).</summary>
    private readonly List<(ulong Id, int Length)> shown = [];
    private int length;
    private ScrollViewer? scroller;
    private bool drawn;

    public LogPane(AppUi ui, PanePage page)
    {
        this.ui = ui;
        var commands = new CommandBar
        {
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            DefaultLabelPosition = CommandBarDefaultLabelPosition.Collapsed,
            IsDynamicOverflowEnabled = false,
            OverflowButtonVisibility = CommandBarOverflowButtonVisibility.Collapsed,
        };
        commands.PrimaryCommands.Add(Command("\uE721", L("Find"), L("Find in log (Ctrl+F)"), () => ShowFind()));
        var clear = Command("\uE74D", L("Clear"), L("Clear the log view (Ctrl+L) — the log file is kept"), Clear);
        clear.KeyboardAccelerators.Add(new KeyboardAccelerator { Key = VirtualKey.L, Modifiers = VirtualKeyModifiers.Control });
        clear.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        commands.PrimaryCommands.Add(clear);
        commands.PrimaryCommands.Add(Command("\uE8C8", L("Copy All"), L("Copy Entire Log"), CopyAll));
        showFile = Command("\uE838", L("Show in File Explorer"), L("Show the log file in File Explorer"), () =>
        {
            if (ui.Config is { } config)
            {
                Cards.ShowInFileExplorer(config.LogFile);
            }
        });
        commands.PrimaryCommands.Add(showFile);
        page.ActionsPanel.Children.Add(commands);

        autoScroll.IsChecked = ui.LogAutoScroll;
        autoScroll.Click += (_, _) =>
        {
            ui.LogAutoScroll = autoScroll.IsChecked == true;
            if (ui.LogAutoScroll)
            {
                ScrollToEnd();
            }
        };
        findBox.KeyDown += (_, args) =>
        {
            if (args.Key == VirtualKey.Enter)
            {
                args.Handled = true;
                FindNext(forward: !InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift).HasFlag(CoreVirtualKeyStates.Down));
            }
            else if (args.Key == VirtualKey.Escape)
            {
                args.Handled = true;
                findBar.Visibility = Visibility.Collapsed;
                view.Focus(FocusState.Keyboard);
            }
        };
        findBox.TextChanged += (_, _) => noMatch.Visibility = Visibility.Collapsed;
        var previous = IconButton("\uE70E", L("Previous match (Shift+Enter)"), () => FindNext(forward: false));
        var next = IconButton("\uE70D", L("Next match (Enter)"), () => FindNext(forward: true));
        findBar.Children.Add(noMatch);
        findBar.Children.Add(findBox);
        findBar.Children.Add(previous);
        findBar.Children.Add(next);

        // On the pane's card, as the other panes' groups, and the found text stays marked while typing in the find box.
        foreach (var key in new[] { "TextControlBackground", "TextControlBackgroundPointerOver", "TextControlBackgroundFocused", "TextControlBorderBrush", "TextControlBorderBrushPointerOver", "TextControlBorderBrushFocused" })
        {
            view.Resources[key] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        }
        view.SelectionHighlightColorWhenNotFocused = (SolidColorBrush)Application.Current.Resources["AccentFillColorDefaultBrush"];
        // The lines are drawn once the view is in the window, whose theme decides their colours, and
        // drawn again when the theme changes, as their colours are fixed in the text.
        view.Loaded += (_, _) =>
        {
            scroller = Cards.FindDescendant(view, "ContentElement") as ScrollViewer;
            Redraw(toEnd: ui.LogAutoScroll);
        };
        view.ActualThemeChanged += (_, _) => Recolor();

        var fill = page.UseFill();
        fill.RowSpacing = 8;
        fill.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        fill.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
        fill.Children.Add(autoScroll);
        fill.Children.Add(findBar);
        var card = new Border { Style = (Style)Application.Current.Resources["ListCardStyle"], Child = view };
        Grid.SetRow(card, 1);
        fill.Children.Add(card);

        ui.LogAppended += Appended;
    }

    public void Dispose() => ui.LogAppended -= Appended;

    public void Refresh() => showFile.IsEnabled = ui.Config is not null;

    /// <summary>The pane's one row; its commands at the top have none.</summary>
    public FrameworkElement? OpenRow(string key) => key == "Auto-scroll" ? autoScroll : null;

    /// <summary>Opens the find box with the keyboard in it.</summary>
    public bool ShowFind()
    {
        findBar.Visibility = Visibility.Visible;
        findBox.Focus(FocusState.Keyboard);
        findBox.SelectAll();
        return true;
    }

    private void Appended(IReadOnlyList<LogLine> lines)
    {
        if (!drawn)
        {
            // Loaded draws every line.
            return;
        }
        var wasAtBottom = scroller is null || scroller.VerticalOffset >= scroller.ScrollableHeight - 8;
        var entries = ui.Log.Entries;
        // Lines the app's log dropped leave the view once there are enough of them, and only while the
        // view is at the end: while the person reads further up, nothing above them moves.
        var stale = entries.Count == 0 ? shown.Count : shown.TakeWhile(entry => entry.Id < entries[0].Id).Count();
        if (stale >= TrimChunk && wasAtBottom)
        {
            var removed = shown.Take(stale).Sum(entry => entry.Length);
            Edit(() => view.Document.GetRange(0, removed).SetText(TextSetOptions.None, ""));
            shown.RemoveRange(0, stale);
            length -= removed;
        }
        Append(ui.Log.After(shown.Count > 0 ? shown[^1].Id : null));
        if (ui.LogAutoScroll && wasAtBottom)
        {
            ScrollToEnd();
        }
    }

    /// <summary>
    /// <c>HH:mm:ss [err|▸] text</c>. stderr carries openconnect's ordinary progress too, so it only gets
    /// a small "err" marker. openconnect has no ERROR:/WARN: prefixes, which macOS colours for openfortivpn.
    /// </summary>
    private void Append(IEnumerable<LogEntry> entries)
    {
        var colors = Palette.For(view.ActualTheme);
        Edit(() =>
        {
            foreach (var entry in entries)
            {
                var start = length;
                var line = entry.Line;
                Run(line.Time.ToLocalTime().ToString("HH:mm:ss", System.Globalization.CultureInfo.InvariantCulture) + " ", colors.Tertiary);
                var text = colors.Primary;
                switch (line.Source)
                {
                    case LogSource.Note:
                        Run("▸ ", colors.Secondary);
                        text = colors.Secondary;
                        break;
                    case LogSource.Stderr:
                        Run("err ", colors.Tertiary, size: 10);
                        break;
                    case LogSource.Stdout:
                        break;
                    default:
                        throw new InvalidOperationException($"unknown log source {line.Source}");
                }
                // The view's line break is one character.
                Run(line.Text.ReplaceLineEndings(" ") + "\r", text);
                shown.Add((entry.Id, length - start));
            }
        });
    }

    /// <summary>Adds <paramref name="text"/> at the end; each run sets its own look, as text added there takes the look before it.</summary>
    private void Run(string text, Color color, float size = 12)
    {
        var range = view.Document.GetRange(length, length);
        range.SetText(TextSetOptions.None, text);
        range.CharacterFormat.ForegroundColor = color;
        range.CharacterFormat.Size = size;
        length += text.Length;
    }

    /// <summary>The view is read-only to the person; the app's own changes lift that for the moment.</summary>
    private void Edit(Action change)
    {
        view.IsReadOnly = false;
        // The view lays out once for the whole change, not once per piece of text.
        view.Document.BatchDisplayUpdates();
        change();
        view.Document.ApplyDisplayUpdates();
        view.IsReadOnly = true;
    }

    private void ScrollToEnd() => view.Document.GetRange(length, length).ScrollIntoView(PointOptions.None);

    /// <summary>
    /// The lines in the new theme's colours. The text is the same, so the place being read and the
    /// found text come back where they were, unless Auto-scroll keeps the view at the end.
    /// </summary>
    private void Recolor()
    {
        var atBottom = scroller is null || scroller.VerticalOffset >= scroller.ScrollableHeight - 8;
        if (ui.LogAutoScroll && atBottom)
        {
            Redraw(toEnd: true);
            return;
        }
        var offset = scroller?.VerticalOffset ?? 0;
        var (start, end) = (view.Document.Selection.StartPosition, view.Document.Selection.EndPosition);
        Redraw(toEnd: false);
        view.Document.Selection.SetRange(start, end);
        view.DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () => scroller?.ChangeView(null, offset, null, disableAnimation: true));
    }

    /// <summary>Every line again, in the current theme's colours, then the end when <paramref name="toEnd"/>.</summary>
    private void Redraw(bool toEnd)
    {
        drawn = true;
        Edit(() => view.Document.SetText(TextSetOptions.None, ""));
        shown.Clear();
        length = 0;
        Append(ui.Log.Entries);
        if (toEnd)
        {
            // After the new text is laid out, or the scroll lands short of the end.
            view.DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, ScrollToEnd);
        }
    }

    private void Clear()
    {
        ui.Log.Clear();
        Edit(() => view.Document.SetText(TextSetOptions.None, ""));
        shown.Clear();
        length = 0;
    }

    private void CopyAll()
    {
        var package = new DataPackage();
        package.SetText(ui.Log.CopyAllText(TimeZoneInfo.Local));
        Clipboard.SetContent(package);
    }

    /// <summary>The next match after the selection, or the one before it, starting over at the other end.</summary>
    private void FindNext(bool forward)
    {
        var query = findBox.Text;
        if (query.Length == 0)
        {
            return;
        }
        var document = view.Document;
        var from = forward ? document.Selection.EndPosition : document.Selection.StartPosition;
        var scope = forward ? TextConstants.MaxUnitCount : -TextConstants.MaxUnitCount;
        var range = document.GetRange(from, from);
        if (range.FindText(query, scope, FindOptions.None) == 0)
        {
            var wrap = forward ? 0 : length;
            range = document.GetRange(wrap, wrap);
            if (range.FindText(query, scope, FindOptions.None) == 0)
            {
                noMatch.Visibility = Visibility.Visible;
                return;
            }
        }
        noMatch.Visibility = Visibility.Collapsed;
        document.Selection.SetRange(range.StartPosition, range.EndPosition);
        range.ScrollIntoView(PointOptions.None);
    }

    private static AppBarButton Command(string glyph, string label, string help, Action action)
    {
        // Four of them fit beside or under the title even in the narrowest window.
        var button = new AppBarButton { Icon = new FontIcon { Glyph = glyph }, Label = label, Width = 56 };
        ToolTipService.SetToolTip(button, help);
        button.Click += (_, _) => action();
        return button;
    }

    private static Button IconButton(string glyph, string help, Action action)
    {
        var button = new Button { Content = new FontIcon { Glyph = glyph, FontSize = 14 }, Width = 36, Height = 32, Padding = new Thickness(0) };
        ToolTipService.SetToolTip(button, help);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, help);
        button.Click += (_, _) => action();
        return button;
    }

    /// <summary>
    /// The log's colours for the theme the lines are added in. The view's text takes no transparency,
    /// so the secondary and tertiary text colours of Fluent are given as the opaque greys they look like.
    /// </summary>
    private sealed record Palette(Color Primary, Color Secondary, Color Tertiary)
    {
        private static readonly Palette Light = new(Hex(0x1B1B1B), Hex(0x5E5E5E), Hex(0x8A8A8A));
        private static readonly Palette Dark = new(Hex(0xFFFFFF), Hex(0xC5C5C5), Hex(0x8A8A8A));

        public static Palette For(ElementTheme theme) => theme == ElementTheme.Dark ? Dark : Light;

        private static Color Hex(int rgb) => Color.FromArgb(0xFF, (byte)(rgb >> 16), (byte)(rgb >> 8), (byte)rgb);
    }
}
