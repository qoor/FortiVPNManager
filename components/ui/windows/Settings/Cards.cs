using CommunityToolkit.WinUI.Controls;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using FortiVPNManager.Ui.Text;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The pieces the panes are built from, in the look of Windows Settings (Styles.xaml): section
/// headers, cards with the setting at the right, captions, and notices.
/// </summary>
internal static class Cards
{
    public static TextBlock SectionHeader(string text, bool first = false) =>
        new() { Text = text, Style = Style(first ? "FirstSectionHeaderStyle" : "SectionHeaderStyle") };

    public static TextBlock Footer(string text) => new() { Text = text, Style = Style("FooterStyle") };

    public static SettingsCard Card(string header, object? content = null) => new() { Header = header, Content = content };

    /// <summary>A card whose buttons start at the left, as the action rows of Windows Settings.</summary>
    public static SettingsCard ActionCard(UIElement content) => new() { Content = content, ContentAlignment = ContentAlignment.Left };

    /// <summary>A value shown at the right of a card; wraps rather than widening the card.</summary>
    public static TextBlock Value() => new() { Style = Style("ValueStyle"), TextWrapping = TextWrapping.Wrap, MaxWidth = 420 };

    /// <summary>
    /// A row of buttons that moves the ones that don't fit to the next line, as when the window is
    /// at its narrowest.
    /// </summary>
    public static WrapPanel Buttons(params UIElement[] children)
    {
        var row = new WrapPanel { HorizontalSpacing = 8, VerticalSpacing = 8 };
        foreach (var child in children)
        {
            row.Children.Add(child);
        }
        return row;
    }

    /// <summary>
    /// Gives <paramref name="field"/> <paramref name="width"/> beside the header, and no more than the
    /// card's inner width once the card puts its setting under the header, which the toolkit does
    /// below its SettingsCardWrapThreshold (476). A fixed width would run past the card's edge there.
    /// </summary>
    public static SettingsCard Fit(this SettingsCard card, FrameworkElement field, double width)
    {
        const double wrapThreshold = 476;
        field.Width = width;
        card.SizeChanged += (_, args) =>
        {
            var inner = args.NewSize.Width - card.Padding.Left - card.Padding.Right;
            field.Width = args.NewSize.Width < wrapThreshold ? Math.Max(0, Math.Min(width, inner)) : width;
        };
        return card;
    }

    public static StackPanel Row(params UIElement[] children)
    {
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        foreach (var child in children)
        {
            row.Children.Add(child);
        }
        return row;
    }

    /// <summary>A notice in the pane: an error, a warning or a success, kept open until the state changes.</summary>
    public static InfoBar Notice(InfoBarSeverity severity) => new() { Severity = severity, IsOpen = true, IsClosable = false, Visibility = Visibility.Collapsed };

    /// <summary>Shows <paramref name="notice"/> with <paramref name="text"/>, or hides it when there is none.</summary>
    public static void Show(this InfoBar notice, string? text)
    {
        notice.Message = text ?? "";
        notice.Visibility = text is null ? Visibility.Collapsed : Visibility.Visible;
    }

    /// <summary>
    /// A status mark and its text, as the success and error lines of the macOS panes. The text wraps
    /// in the room it has rather than running past a narrow card.
    /// </summary>
    public static (Grid Line, FontIcon Mark, TextBlock Text) StatusLine()
    {
        var mark = new FontIcon { FontSize = 14, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 3, 0, 0) };
        var text = new TextBlock { TextWrapping = TextWrapping.Wrap, MaxWidth = 420, IsTextSelectionEnabled = true };
        var line = new Grid { ColumnSpacing = 6, VerticalAlignment = VerticalAlignment.Center };
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        Grid.SetColumn(text, 1);
        line.Children.Add(mark);
        line.Children.Add(text);
        return (line, mark, text);
    }

    public static void SetSucceeded(this FontIcon mark, bool succeeded) => mark.SetStyle(succeeded ? NoticeStyle.Success : NoticeStyle.Error);

    public static void SetStyle(this FontIcon mark, NoticeStyle style) =>
        (mark.Glyph, mark.Foreground) = style switch
        {
            NoticeStyle.Success => ("\uEC61", Brush("SystemFillColorSuccessBrush")),
            NoticeStyle.Info => ("\uE946", Brush("TextFillColorSecondaryBrush")),
            NoticeStyle.Warning => ("\uE7BA", Brush("SystemFillColorCautionBrush")),
            NoticeStyle.Error => ("\uEA39", Brush("SystemFillColorCriticalBrush")),
            _ => throw new InvalidOperationException($"unknown notice style {style}"),
        };

    /// <summary>Opens File Explorer with <paramref name="path"/> selected.</summary>
    public static void ShowInFileExplorer(string path) =>
        System.Diagnostics.Process.Start("explorer.exe", $"/select,\"{path}\"");

    /// <summary>Hidden buttons leave the layout; the others are dimmed when they don't apply now.</summary>
    public static void Apply(this Button button, ButtonState state)
    {
        button.Visibility = state == ButtonState.Hidden ? Visibility.Collapsed : Visibility.Visible;
        button.IsEnabled = state == ButtonState.Enabled;
    }

    /// <summary>The caption under a card's header, or none.</summary>
    public static void Describe(this SettingsCard card, string? text)
    {
        if (text is null)
        {
            card.ClearValue(SettingsCard.DescriptionProperty);
        }
        else
        {
            card.Description = text;
        }
    }

    public static void Visible(this UIElement element, bool visible) =>
        element.Visibility = visible ? Visibility.Visible : Visibility.Collapsed;

    /// <summary>A named part inside a control's template, to adjust what the control offers no setting for.</summary>
    public static DependencyObject? FindDescendant(DependencyObject parent, string name)
    {
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
        {
            var child = VisualTreeHelper.GetChild(parent, i);
            if (child is FrameworkElement { Name: var childName } && childName == name)
            {
                return child;
            }
            if (FindDescendant(child, name) is { } found)
            {
                return found;
            }
        }
        return null;
    }

    public static Brush Brush(string key) => (Brush)Application.Current.Resources[key];

    private static Style Style(string key) => (Style)Application.Current.Resources[key];
}
