using System.Runtime.InteropServices;
using System.Text.Json;
using FortiVPNManager.Contract;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Navigation;
using Windows.Graphics;
using Windows.System;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The settings window, laid out as Windows Settings (docs/SETTINGS-UI.md): the panes in the left
/// navigation, the back button and the settings search in the title bar. Below 800 px of width the
/// navigation hides behind the menu button and the search becomes a magnifier after the title.
/// Closing hides the window; the app stays in the notification area. It reopens on the pane and at
/// the place it was left.
/// </summary>
public sealed partial class SettingsWindow : Window
{
    /// <summary>The panes' Segoe Fluent Icons glyphs, in the navigation's order.</summary>
    private static readonly (SettingsPane Pane, string Glyph)[] Panes =
    [
        (SettingsPane.General, "\uE713"),
        (SettingsPane.Account, "\uE77B"),
        (SettingsPane.Connection, "\uE968"),
        (SettingsPane.ExceptionRules, "\uE8AB"),
        (SettingsPane.Advanced, "\uE90F"),
        (SettingsPane.Log, "\uE8A5"),
    ];

    /// <summary>Where the window's place and last pane are kept between runs.</summary>
    private static readonly string PlacementFile = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "FortiVPNManager", "settings-window.json");

    private readonly AppUi ui;
    private bool narrow;
    private bool shown;
    private SettingsPane lastPane;

    public SettingsWindow(AppUi ui)
    {
        this.ui = ui;
        InitializeComponent();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(AppTitleBar);
        // The app icon in the taskbar and Alt+Tab; the title bar shows none, as Windows Settings.
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "AppIcon.ico"));
        Search.PlaceholderText = L("Search");
        FlyoutSearch.PlaceholderText = L("Search");
        ToolTipService.SetToolTip(NarrowSearchButton, L("Search"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(NarrowSearchButton, L("Search"));
        foreach (var (pane, glyph) in Panes)
        {
            Navigation.MenuItems.Add(new NavigationViewItem { Content = L(pane.TitleKey()), Tag = pane, Icon = new FontIcon { Glyph = glyph } });
        }
        // A result shows its own text; keyboard selection in the list leaves the typed term as it is.
        Search.UpdateTextOnSelect = false;
        FlyoutSearch.UpdateTextOnSelect = false;
        AddKeyboardShortcuts();
        var placement = Placement.Load();
        lastPane = placement?.Pane ?? SettingsPane.General;
        Place(placement);
        SizeChanged += (_, args) => ApplyWidth(args.Size.Width);
        ApplyWidth(AppWindow.Size.Width / Scale);
        AppWindow.Closing += (_, args) =>
        {
            args.Cancel = true;
            Hide();
        };
    }

    /// <summary>Shows the window on <paramref name="pane"/>, or on the pane last shown.</summary>
    public void Show(SettingsPane? pane)
    {
        Select(pane ?? lastPane);
        // A minimized window comes back, so the menu's Settings… or a notification never looks ignored.
        if (AppWindow.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized } presenter)
        {
            presenter.Restore();
        }
        AppWindow.Show();
        Activate();
        if (!shown)
        {
            shown = true;
            ui.Send(new AppInternal.Command.SettingsOpened());
        }
    }

    /// <summary>Hides the window and keeps where it was and the pane shown.</summary>
    public void Hide()
    {
        if (!shown)
        {
            return;
        }
        shown = false;
        // What is still being typed is saved first, as closing takes the field away before it loses focus.
        FinishTyping();
        new Placement(lastPane, AppWindow.Position.X, AppWindow.Position.Y, AppWindow.Size.Width, AppWindow.Size.Height).Save();
        AppWindow.Hide();
        ui.Send(new AppInternal.Command.SettingsClosed());
    }

    /// <summary>Sends what the shown pane still has in its fields, as before the window closes or the app quits.</summary>
    public void FinishTyping() => (ContentFrame.Content as PanePage)?.FinishTyping();

    private double Scale => GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(this)) / 96.0;

    private void Select(SettingsPane pane)
    {
        var item = Navigation.MenuItems.OfType<NavigationViewItem>().First(i => (SettingsPane)i.Tag == pane);
        if (Navigation.SelectedItem != (object)item)
        {
            Navigation.SelectedItem = item;
        }
    }

    private void OnSelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (args.SelectedItem is NavigationViewItem { Tag: SettingsPane pane }
            && (ContentFrame.Content as PanePage)?.Pane != pane)
        {
            ContentFrame.Navigate(typeof(PanePage), pane);
        }
    }

    /// <summary>The window's title stays the app's name, as Windows Settings keeps its own; the back button follows the history.</summary>
    private void OnNavigated(object sender, NavigationEventArgs e)
    {
        var pane = (SettingsPane)e.Parameter;
        ((PanePage)e.Content).Show(ui, pane, L(pane.TitleKey()));
        lastPane = pane;
        AppTitleBar.IsBackButtonEnabled = ContentFrame.CanGoBack;
        Select(pane);
    }

    private void OnBackRequested(TitleBar sender, object args) => GoBack();

    private void GoBack()
    {
        if (ContentFrame.CanGoBack)
        {
            ContentFrame.GoBack();
        }
    }

    /// <summary>
    /// The results of the settings search under the box, as in Windows Settings: each setting with the
    /// pane it is in. Typing changes nothing on the right.
    /// </summary>
    private void OnSearchTextChanged(AutoSuggestBox sender, AutoSuggestBoxTextChangedEventArgs args)
    {
        RoundClearButton(sender);
        if (args.Reason != AutoSuggestionBoxTextChangeReason.UserInput)
        {
            return;
        }
        if (sender.Text.Trim().Length == 0)
        {
            sender.ItemsSource = null;
            return;
        }
        var results = SettingsSearch.Results(sender.Text, ui.IsShown);
        // The input method finishes the syllable being composed when Down moves into the list, which
        // reports the text again; the same results stay as they are, so the line just chosen stays chosen.
        var shown = (sender.ItemsSource as List<object>)?.Select(line => (line as FrameworkElement)?.Tag as SearchResult).ToList();
        if (shown is not null && (results.Count == 0 ? shown is [null] : shown.SequenceEqual(results)))
        {
            return;
        }
        sender.ItemsSource = results.Count == 0
            ? new List<object> { new TextBlock { Text = L("No results"), Foreground = Cards.Brush("TextFillColorSecondaryBrush") } }
            : results.Select(Suggestion).ToList<object>();
    }

    /// <summary>A result's line: the pane's glyph, the setting's name (up to three lines) and, for a setting, its pane.</summary>
    private static Grid Suggestion(SearchResult result)
    {
        var line = new Grid { ColumnSpacing = 12, Tag = result, Padding = new Thickness(0, 4, 0, 4) };
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        line.Children.Add(new FontIcon { Glyph = Panes.First(p => p.Pane == result.Pane).Glyph, FontSize = 16, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 2, 0, 0) });
        var text = new StackPanel();
        text.Children.Add(new TextBlock { Text = result.Item?.Title ?? L(result.Pane.TitleKey()), TextWrapping = TextWrapping.Wrap, MaxLines = 3 });
        if (result.Item is not null)
        {
            text.Children.Add(new TextBlock { Text = L(result.Pane.TitleKey()), Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"], Foreground = Cards.Brush("TextFillColorSecondaryBrush") });
        }
        Grid.SetColumn(text, 1);
        line.Children.Add(text);
        return line;
    }

    /// <summary>A chosen result, or Enter for the first one; the keyboard stays in the box after Enter.</summary>
    private void OnSearchQuerySubmitted(AutoSuggestBox sender, AutoSuggestBoxQuerySubmittedEventArgs args)
    {
        var result = (args.ChosenSuggestion as FrameworkElement)?.Tag as SearchResult
            ?? SettingsSearch.Results(sender.Text, ui.IsShown).FirstOrDefault();
        if (result is null)
        {
            return;
        }
        SearchFlyout.Hide();
        Reveal(result);
    }

    /// <summary>
    /// Goes to the result's pane; for a setting, opens the group it is in, scrolls to its row and
    /// highlights it for a moment, once the pane is laid out.
    /// </summary>
    private void Reveal(SearchResult result)
    {
        Select(result.Pane);
        if (result.Item?.Row is { } row)
        {
            DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () => (ContentFrame.Content as PanePage)?.Reveal(row));
        }
    }

    /// <summary>
    /// The clear button marks the pointer with a round background 28 wide and 24 tall, its cross 18 px
    /// in from the box's end, as measured on the search box of Windows Settings; the template draws a
    /// rounded square 32 wide. The button exists once the box has text, which is when it first shows.
    /// </summary>
    private static void RoundClearButton(AutoSuggestBox box)
    {
        if (Cards.FindDescendant(box, "DeleteButton") is not Button button || button.MaxWidth == 28)
        {
            return;
        }
        // The text box puts the button's own width back, so a maximum width holds it at 28.
        button.MaxWidth = 28;
        // The button's own template comes once it first shows, when it is laid out at its size.
        void Round(object sender, SizeChangedEventArgs e)
        {
            if (Cards.FindDescendant(button, "ButtonLayoutGrid") is Grid background)
            {
                background.CornerRadius = new CornerRadius(12);
                button.SizeChanged -= Round;
            }
        }
        button.SizeChanged += Round;
    }

    private Flyout SearchFlyout => (Flyout)RootGrid.Resources["SearchFlyout"];

    /// <summary>
    /// The window's width decides the layout before the title bar lays out its content, which hides
    /// the title when the content does not fit. From 800 px the navigation stays beside the pages
    /// and the search box keeps the 516 px of Windows Settings, narrowing so it never reaches the
    /// back button and the app's name (about 230 px with the gap) or, kept centered, the caption
    /// buttons. Below 800 px the navigation hides and the search becomes a magnifier after the title.
    /// </summary>
    private void ApplyWidth(double width)
    {
        const double reserved = 230;
        narrow = width < 800;
        SearchArea.Width = Math.Clamp(width - 2 * reserved, 200, 516);
        Navigation.PaneDisplayMode = narrow ? NavigationViewPaneDisplayMode.LeftMinimal : NavigationViewPaneDisplayMode.Left;
        ApplyWidthLayout();
    }

    private void OnTitleBarLoaded(object sender, RoutedEventArgs e) => ApplyWidthLayout();

    private void ApplyWidthLayout()
    {
        AppTitleBar.IsPaneToggleButtonVisible = narrow;
        SearchArea.Visibility = narrow ? Visibility.Collapsed : Visibility.Visible;
        NarrowSearchButton.Visibility = narrow ? Visibility.Visible : Visibility.Collapsed;
        // The title bar centers its content; the magnifier sits right after the title instead.
        if (Cards.FindDescendant(AppTitleBar, "PART_ContentPresenter") is FrameworkElement presenter)
        {
            presenter.HorizontalAlignment = narrow ? HorizontalAlignment.Left : HorizontalAlignment.Center;
        }
        if (!narrow)
        {
            SearchFlyout.Hide();
        }
    }

    private void OnPaneToggleRequested(TitleBar sender, object args)
    {
        SearchFlyout.Hide();
        Navigation.IsPaneOpen = !Navigation.IsPaneOpen;
    }

    /// <summary>The flyout opens under the title bar near the window's left edge, as in Windows Settings.</summary>
    private void OnNarrowSearchClick(object sender, RoutedEventArgs e) => OpenNarrowSearch();

    private void OpenNarrowSearch() =>
        SearchFlyout.ShowAt(RootGrid, new FlyoutShowOptions { Position = new Windows.Foundation.Point(22, 46), Placement = FlyoutPlacementMode.BottomEdgeAlignedLeft });

    private void OnSearchFlyoutOpened(object? sender, object e) => FlyoutSearch.Focus(FocusState.Programmatic);

    /// <summary>Windows conventions: Alt+Left goes back, Ctrl+F goes to the settings search (on the Log pane, to the log's find).</summary>
    private void AddKeyboardShortcuts()
    {
        var back = new KeyboardAccelerator { Key = VirtualKey.Left, Modifiers = VirtualKeyModifiers.Menu };
        back.Invoked += (_, args) =>
        {
            GoBack();
            args.Handled = true;
        };
        var find = new KeyboardAccelerator { Key = VirtualKey.F, Modifiers = VirtualKeyModifiers.Control };
        find.Invoked += (_, args) =>
        {
            args.Handled = true;
            // The Log pane finds in the log, as Ctrl+F does in a document.
            if (ContentFrame.Content is PanePage page && page.Find())
            {
                return;
            }
            if (narrow)
            {
                OpenNarrowSearch();
            }
            else
            {
                Search.Focus(FocusState.Keyboard);
            }
        };
        RootGrid.KeyboardAccelerators.Add(back);
        RootGrid.KeyboardAccelerators.Add(find);
        // The shortcuts are known conventions; a tooltip naming them over the whole window only gets in the way.
        RootGrid.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
    }

    /// <summary>
    /// The saved place when it is still on a display, as Windows Settings reopens where it was left;
    /// otherwise 1040 x 720 centered on the primary display. Never narrower than Windows Settings (348).
    /// </summary>
    private void Place(Placement? placement)
    {
        var scale = Scale;
        var rect = placement is { } saved ? new RectInt32(saved.X, saved.Y, saved.Width, saved.Height) : default;
        var area = DisplayArea.GetFromRect(rect, DisplayAreaFallback.None);
        if (placement is null || area is null)
        {
            var work = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
            var size = new SizeInt32((int)(1040 * scale), (int)(720 * scale));
            rect = new RectInt32(work.X + (work.Width - size.Width) / 2, work.Y + (work.Height - size.Height) / 2, size.Width, size.Height);
        }
        AppWindow.MoveAndResize(rect);
        if (AppWindow.Presenter is OverlappedPresenter presenter)
        {
            presenter.PreferredMinimumWidth = (int)(348 * scale);
            presenter.PreferredMinimumHeight = (int)(400 * scale);
        }
    }

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr window);

    /// <summary>The window's place in physical pixels and the pane last shown.</summary>
    private sealed record Placement(SettingsPane Pane, int X, int Y, int Width, int Height)
    {
        public static Placement? Load()
        {
            try
            {
                var placement = JsonSerializer.Deserialize<Placement>(File.ReadAllText(PlacementFile));
                // A pane number this build doesn't have, from an edited or older file, opens General.
                return placement is { } read && !Enum.IsDefined(read.Pane) ? read with { Pane = SettingsPane.General } : placement;
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException or JsonException)
            {
                return null;
            }
        }

        public void Save()
        {
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(PlacementFile)!);
                File.WriteAllText(PlacementFile, JsonSerializer.Serialize(this));
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            {
                // Only the place is lost; the window opens centered next time.
            }
        }
    }
}
