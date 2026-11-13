using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Navigation;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// One pane of the settings window, under its title. The pane's settings follow the runtime's
/// sections while the pane is shown.
/// </summary>
public sealed partial class PanePage : Page
{
    private AppUi? ui;
    private IPane? content;

    public PanePage()
    {
        InitializeComponent();
    }

    public SettingsPane? Pane { get; private set; }

    internal StackPanel BodyPanel => Body;

    internal StackPanel ActionsPanel => Actions;

    /// <summary>The whole area under the title, for a pane that scrolls on its own; the settings area goes away.</summary>
    internal Grid UseFill()
    {
        Scroller.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
        Fill.Visibility = Microsoft.UI.Xaml.Visibility.Visible;
        return Fill;
    }

    /// <summary>Ctrl+F: the log's own find, on the Log pane; false elsewhere, where it goes to the settings search.</summary>
    internal bool Find() => content is LogPane log && log.ShowFind();

    /// <summary>Saves what is still being typed, before the pane or the window goes away.</summary>
    internal void FinishTyping() => content?.FinishTyping();

    public void Show(AppUi ui, SettingsPane pane, string title)
    {
        this.ui = ui;
        Pane = pane;
        Title.Text = title;
        content = pane switch
        {
            SettingsPane.General => new GeneralPane(ui, this),
            SettingsPane.Account => new AccountPane(ui, this),
            SettingsPane.Connection => new ConnectionPane(ui, this),
            SettingsPane.ExceptionRules => new ExceptionRulesPane(ui, this),
            SettingsPane.Advanced => new AdvancedPane(ui, this),
            SettingsPane.Log => new LogPane(ui, this),
            _ => throw new InvalidOperationException($"unknown pane {pane}"),
        };
        Refresh();
        ui.Changed += Refresh;
    }

    protected override void OnNavigatedFrom(NavigationEventArgs e)
    {
        base.OnNavigatedFrom(e);
        FinishTyping();
        if (ui is not null)
        {
            ui.Changed -= Refresh;
        }
        (content as IDisposable)?.Dispose();
    }

    private void Refresh() => content?.Refresh();

    /// <summary>
    /// Below 560 px, about the narrow window's width, the buttons move under the title, where the Log
    /// pane's four commands or Apply and Discard Changes leave the title room.
    /// </summary>
    private void OnSizeChanged(object sender, Microsoft.UI.Xaml.SizeChangedEventArgs e)
    {
        var stacked = e.NewSize.Width < 560;
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
}
