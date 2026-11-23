using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// One pane of the settings window, under its title. The pane's settings follow the runtime's
/// sections while the pane is shown.
/// </summary>
public sealed partial class PanePage : Page
{
    private AppUi? _ui;
    private IPane? _content;

    public PanePage() => InitializeComponent();

    public SettingsPane? Pane { get; private set; }

    public SettingsSubpage? Subpage { get; private set; }

    internal StackPanel BodyPanel => Body;

    internal StackPanel ActionsPanel => Actions;

    /// <summary>The whole area under the title, for a pane that scrolls on its own; the settings area goes away.</summary>
    internal Grid UseFill()
    {
        Scroller.Visibility = Visibility.Collapsed;
        Fill.Visibility = Visibility.Visible;
        return Fill;
    }

    /// <summary>Ctrl+F: the log's own find, on the Log pane; false elsewhere, where it goes to the settings search.</summary>
    internal bool Find() => _content is LogPane log && log.ShowFind();

    /// <summary>Saves what is still being typed, before the pane or the window goes away.</summary>
    internal void FinishTyping() => _content?.FinishTyping();

    /// <summary>
    /// A search result's row: the pane opens the expander it is in, then, once the row is laid out (a
    /// row in an expander just opened appears a moment later), it comes into view and is highlighted for
    /// two seconds.
    /// </summary>
    internal void Reveal(string row)
    {
        if (_content?.OpenRow(row) is not { } element)
        {
            return;
        }
        if (element.IsLoaded)
        {
            Show(element);
            return;
        }
        void Loaded(object sender, RoutedEventArgs e)
        {
            element.Loaded -= Loaded;
            Show(element);
        }
        element.Loaded += Loaded;
    }

    private static void Show(FrameworkElement element)
    {
        element.StartBringIntoView(new BringIntoViewOptions { VerticalAlignmentRatio = 0.3, AnimationDesired = true });
        Highlight(element);
    }

    /// <summary>Rows highlighted now, each with the timer that ends its highlight.</summary>
    private static readonly Dictionary<FrameworkElement, DispatcherQueueTimer> s_highlighted = [];

    /// <summary>
    /// The row's border in the accent colour for two seconds, as Windows Settings marks the setting a
    /// search went to; its own border, from its style, comes back after. Choosing it again meanwhile
    /// starts the two seconds over.
    /// </summary>
    private static void Highlight(FrameworkElement element)
    {
        if (s_highlighted.TryGetValue(element, out DispatcherQueueTimer? running))
        {
            running.Stop();
            running.Start();
            return;
        }
        (DependencyProperty? brush, DependencyProperty? thickness) = element switch
        {
            Control => (Control.BorderBrushProperty, Control.BorderThicknessProperty),
            Border => (Border.BorderBrushProperty, Border.BorderThicknessProperty),
            _ => (null, null),
        };
        if (brush is null || thickness is null)
        {
            return;
        }
        element.SetValue(brush, Application.Current.Resources["AccentFillColorDefaultBrush"]);
        element.SetValue(thickness, new Thickness(2));
        DispatcherQueueTimer timer = element.DispatcherQueue.CreateTimer();
        timer.Interval = TimeSpan.FromSeconds(2);
        timer.IsRepeating = false;
        timer.Tick += (_, _) =>
        {
            element.ClearValue(brush);
            element.ClearValue(thickness);
            _ = s_highlighted.Remove(element);
        };
        s_highlighted[element] = timer;
        timer.Start();
    }

    public void Show(AppUi ui, SettingsPane pane, string title)
    {
        ArgumentNullException.ThrowIfNull(ui);
        Title.Text = title;
        _content = pane switch
        {
            SettingsPane.General => new GeneralPane(ui, this),
            SettingsPane.Account => new AccountPane(ui, this),
            SettingsPane.Connection => new ConnectionPane(ui, this),
            SettingsPane.ExceptionRules => new ExceptionRulesPane(ui, this),
            SettingsPane.Advanced => new AdvancedPane(ui, this),
            SettingsPane.Log => new LogPane(ui, this),
            _ => throw new InvalidOperationException($"unknown pane {pane}"),
        };
        Show(ui, pane);
    }

    public void Show(AppUi ui, SettingsSubpage subpage)
    {
        ArgumentNullException.ThrowIfNull(ui);
        Subpage = subpage;
        Title.Visibility = Visibility.Collapsed;
        Breadcrumb.Visibility = Visibility.Visible;
        Breadcrumb.ItemsSource = new List<object> { L(SettingsPane.General.TitleKey()), L("Software Update") };
        Breadcrumb.ItemClicked += (_, _) => Frame.GoBack();
        _content = subpage switch
        {
            SettingsSubpage.SoftwareUpdate => new UpdatePane(ui, this),
            _ => throw new InvalidOperationException($"unknown subpage {subpage}"),
        };
        Show(ui, SettingsPane.General);
    }

    private void Show(AppUi ui, SettingsPane pane)
    {
        this._ui = ui;
        Pane = pane;
        Refresh();
        ui.Changed += Refresh;
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        Leave();
    }

    /// <summary>
    /// The pane goes away, as another is chosen or the window closes: what is typed is sent, and the
    /// app's state no longer holds the pane.
    /// </summary>
    internal void Leave()
    {
        FinishTyping();
        _ui?.Changed -= Refresh;
        (_content as IDisposable)?.Dispose();
        _content = null;
    }

    private void Refresh() => _content?.Refresh();

    /// <summary>
    /// Below 560 px, about the narrow window's width, the buttons move under the title, where the Log
    /// pane's four commands leave the title room.
    /// </summary>
    private void OnSizeChanged(object sender, SizeChangedEventArgs e)
    {
        bool stacked = e.NewSize.Width < 560;
        Grid.SetRow(Actions, stacked ? 1 : 0);
        Grid.SetColumn(Actions, stacked ? 0 : 1);
    }
}

/// <summary>A pane's settings, drawn again from the runtime's latest sections on every change.</summary>
internal interface IPane
{
    void Refresh();

    /// <summary>Sends a value still being typed, which would otherwise wait for its field to lose focus.</summary>
    void FinishTyping()
    {
    }

    /// <summary>
    /// The row a search result names (its catalog key), if the pane shows it now, with the expander it
    /// is in opened.
    /// </summary>
    FrameworkElement? OpenRow(string key) => null;
}
