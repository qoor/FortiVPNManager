using System.Runtime.InteropServices;
using FortiVPNManager.Contract.AppInternal;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics.Imaging;
using Windows.Storage;
using Windows.Storage.Streams;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

internal sealed class UpdatePane : IPane
{
    private const double IconSize = 48;
    private const int RoundToNearestDisplayedDigit = 1;
    private readonly AppUi _ui;
    private readonly StackPanel _body = new() { Spacing = 4 };
    private readonly ProgressRing _busy = new() { Width = IconSize, Height = IconSize };
    private readonly FontIcon _upToDate = new() { Glyph = "\uEC61", FontSize = IconSize, Foreground = Cards.Brush("SystemFillColorSuccessBrush") };
    private readonly Image _appIcon = new() { Width = IconSize, Height = IconSize };
    private readonly TextBlock _headline = new() { Style = (Style)Application.Current.Resources["SubtitleTextBlockStyle"], TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock _subtitle = new() { Foreground = Cards.Brush("TextFillColorSecondaryBrush"), TextWrapping = TextWrapping.Wrap };
    private readonly Button _check = new() { Content = L("Check for updates") };
    private readonly Button _updateNow = new() { Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
    private readonly Button _restart = new() { Content = L("Restart app"), Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
    private readonly InfoBar _failure = Cards.Notice(InfoBarSeverity.Error);
    private readonly Border _item = new() { Style = (Style)Application.Current.Resources["UpdateCardStyle"] };
    private readonly TextBlock _itemName = new() { TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center };
    private readonly TextBlock _itemStatus = new() { Foreground = Cards.Brush("TextFillColorSecondaryBrush"), VerticalAlignment = VerticalAlignment.Center };
    private readonly Button _cancel = new() { Content = L("Cancel") };
    private readonly ProgressBar _bar = new() { Maximum = 100 };
    private readonly TextBlock _details = new() { Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"], Foreground = Cards.Brush("TextFillColorSecondaryBrush"), TextWrapping = TextWrapping.Wrap };
    private readonly Border _notes = new() { Style = (Style)Application.Current.Resources["UpdateCardStyle"] };
    private readonly StackPanel _whatsNew = new() { Spacing = 4 };
    private readonly StackPanel _noteTitles = new() { Spacing = 4 };
    private readonly HyperlinkButton _releasePage = new() { Content = L("View on GitHub"), Padding = new Thickness(0), Margin = new Thickness(0, 6, 0, 0) };
    private readonly TextBlock _footnote = Cards.Footer(L("The VPN disconnects briefly during the install."));
    private IReadOnlyList<string> _shownNotes = [];

    public UpdatePane(AppUi ui, PanePage page)
    {
        this._ui = ui;
        _check.Click += (_, _) => ui.Send(new CommandCheckForUpdates());
        _updateNow.Click += (_, _) => ui.Send(new CommandUpdateNow());
        _restart.Click += (_, _) => ui.Send(new CommandRestartToUpdate());
        _cancel.Click += (_, _) => ui.Send(new CommandCancelUpdateDownload());
        _appIcon.Loaded += (_, _) => ShowIcon();

        var icons = new Grid { Width = IconSize, Height = IconSize, VerticalAlignment = VerticalAlignment.Center };
        icons.Children.Add(_busy);
        icons.Children.Add(_upToDate);
        icons.Children.Add(_appIcon);
        var text = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        text.Children.Add(_headline);
        text.Children.Add(_subtitle);
        var actions = new Grid { VerticalAlignment = VerticalAlignment.Center };
        actions.Children.Add(_check);
        actions.Children.Add(_updateNow);
        actions.Children.Add(_restart);
        var hero = new Grid { ColumnSpacing = 16, Margin = new Thickness(0, 4, 0, 14) };
        hero.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        hero.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        hero.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        Grid.SetColumn(text, 1);
        Grid.SetColumn(actions, 2);
        hero.Children.Add(icons);
        hero.Children.Add(text);
        hero.Children.Add(actions);

        var itemLine = new Grid { ColumnSpacing = 12 };
        itemLine.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        itemLine.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        itemLine.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        Grid.SetColumn(_itemStatus, 1);
        Grid.SetColumn(_cancel, 2);
        itemLine.Children.Add(_itemName);
        itemLine.Children.Add(_itemStatus);
        itemLine.Children.Add(_cancel);
        var itemBody = new StackPanel { Spacing = 8 };
        itemBody.Children.Add(itemLine);
        itemBody.Children.Add(_bar);
        itemBody.Children.Add(_details);
        _item.Child = itemBody;

        _whatsNew.Children.Add(new TextBlock { Text = L("What's new"), Style = (Style)Application.Current.Resources["BodyStrongTextBlockStyle"], Margin = new Thickness(0, 0, 0, 4) });
        _whatsNew.Children.Add(_noteTitles);
        var notesBody = new StackPanel();
        notesBody.Children.Add(_whatsNew);
        notesBody.Children.Add(_releasePage);
        _notes.Child = notesBody;

        foreach (UIElement element in new UIElement[] { hero, _failure, _item, _notes, _footnote })
        {
            _body.Children.Add(element);
        }
        page.BodyPanel.Children.Add(_body);
        ui.Send(new CommandUpdatePageOpened());
    }

    public void Refresh()
    {
        _body.Visible(_ui.AutoUpdate is not null);
        if (_ui.AutoUpdate is not { } update)
        {
            return;
        }
        UpdateProgress progress = update.Progress;
        bool settled = progress is UpdateProgressIdle or UpdateProgressFailed;
        var available = update.Status as UpdateStatusAvailable;

        bool spinning = settled ? update.Status is UpdateStatusChecking : progress is UpdateProgressInstalling;
        bool current = settled && update.Status is UpdateStatusUpToDate;
        _busy.IsActive = spinning;
        _upToDate.Visible(current);
        _appIcon.Visible(!spinning && !current);
        _headline.Text = update.Headline();
        string? line = update.Subtitle();
        _subtitle.Visible(line is not null);
        _subtitle.Text = line ?? "";
        Button action = settled
            ? available is not null ? _updateNow : _check
            : progress is UpdateProgressDownloading or UpdateProgressPreparing ? _updateNow : _restart;
        _check.Apply(action == _check ? update.Check : ButtonState.Hidden);
        _updateNow.Apply(action == _updateNow ? update.UpdateNow : ButtonState.Hidden);
        _restart.Apply(action == _restart ? update.Restart : ButtonState.Hidden);
        _updateNow.Content = progress is UpdateProgressFailed ? L("Try again") : L("Install now");

        string? failedInstall = update.FailedInstallLine();
        string? reason = (progress as UpdateProgressFailed)?.Reason.Message();
        _failure.Title = failedInstall ?? "";
        _failure.Message = reason ?? "";
        _failure.Visible(failedInstall is not null || reason is not null);

        string? status = progress.ItemStatus();
        _item.Visible(status is not null);
        _itemStatus.Text = status ?? "";
        _itemName.Text = progress switch
        {
            UpdateProgressDownloading or UpdateProgressPreparing => $"FortiVPNManager {available!.Version}",
            UpdateProgressReadyToRestart ready => $"FortiVPNManager {ready.Version}",
            UpdateProgressInstalling installing => $"FortiVPNManager {installing.Version}",
            _ => "",
        };
        _cancel.Apply(update.Cancel);
        byte? percent = progress switch
        {
            UpdateProgressDownloading downloading => downloading.Percent,
            UpdateProgressPreparing preparing => preparing.Percent,
            _ => null,
        };
        _bar.Visible(percent is not null);
        _bar.Value = percent ?? 0;
        var download = progress as UpdateProgressDownloading;
        _details.Visible(download is not null);
        _details.Text = download is null ? "" : $"{Size(download.ReceivedBytes)} / {Size(download.TotalBytes)} · {download.Remaining.Line()}";

        _notes.Visible(settled && available is not null);
        _footnote.Visible(settled && available is not null);
        if (settled && available is not null)
        {
            _releasePage.NavigateUri = available.ReleasePageUrl;
            ShowNotes(available.ReleaseNotes() ?? []);
        }
    }

    private void ShowNotes(IReadOnlyList<string> titles)
    {
        _whatsNew.Visible(titles.Count > 0);
        if (titles.SequenceEqual(_shownNotes))
        {
            return;
        }
        _shownNotes = titles;
        _noteTitles.Children.Clear();
        foreach (string title in titles)
        {
            var line = new Grid { ColumnSpacing = 8 };
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var text = new TextBlock { Text = title, TextWrapping = TextWrapping.Wrap };
            Grid.SetColumn(text, 1);
            line.Children.Add(new TextBlock { Text = "•" });
            line.Children.Add(text);
            _noteTitles.Children.Add(line);
        }
    }

    private async void ShowIcon()
    {
        uint size = (uint)Math.Round(IconSize * _appIcon.XamlRoot.RasterizationScale);
        using IRandomAccessStream stream = await FileRandomAccessStream.OpenAsync(Path.Combine(AppContext.BaseDirectory, "AppIcon.ico"), FileAccessMode.Read);
        BitmapDecoder decoder = await BitmapDecoder.CreateAsync(stream);
        var frames = new List<BitmapFrame>();
        for (uint i = 0; i < decoder.FrameCount; i++)
        {
            frames.Add(await decoder.GetFrameAsync(i));
        }
        BitmapFrame frame = frames.Where(f => f.PixelWidth >= size).MinBy(f => f.PixelWidth) ?? frames.MaxBy(f => f.PixelWidth)!;
        var transform = new BitmapTransform { ScaledWidth = size, ScaledHeight = size, InterpolationMode = BitmapInterpolationMode.Fant };
        SoftwareBitmap bitmap = await frame.GetSoftwareBitmapAsync(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied, transform, ExifOrientationMode.IgnoreExifOrientation, ColorManagementMode.DoNotColorManage);
        var source = new SoftwareBitmapSource();
        await source.SetBitmapAsync(bitmap);
        _appIcon.Source = source;
    }

    private static string Size(ulong bytes)
    {
        char[] buffer = new char[32];
        Marshal.ThrowExceptionForHR(StrFormatByteSizeEx(bytes, RoundToNearestDisplayedDigit, buffer, (uint)buffer.Length));
        return new string(buffer, 0, Array.IndexOf(buffer, '\0'));
    }

    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int StrFormatByteSizeEx(ulong size, int flags, [Out] char[] buffer, uint length);
}
