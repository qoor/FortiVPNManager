using System.Runtime.InteropServices;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics.Imaging;
using Windows.Storage;
using Windows.Storage.Streams;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

internal sealed class UpdatePane : IPane
{
    private const double IconSize = 48;
    private const int RoundToNearestDisplayedDigit = 1;
    private readonly AppUi ui;
    private readonly StackPanel body = new() { Spacing = 4 };
    private readonly ProgressRing busy = new() { Width = IconSize, Height = IconSize };
    private readonly FontIcon upToDate = new() { Glyph = "\uEC61", FontSize = IconSize, Foreground = Cards.Brush("SystemFillColorSuccessBrush") };
    private readonly Image appIcon = new() { Width = IconSize, Height = IconSize };
    private readonly TextBlock headline = new() { Style = (Style)Application.Current.Resources["SubtitleTextBlockStyle"], TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock subtitle = new() { Foreground = Cards.Brush("TextFillColorSecondaryBrush"), TextWrapping = TextWrapping.Wrap };
    private readonly Button check = new() { Content = L("Check for updates") };
    private readonly Button updateNow = new() { Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
    private readonly Button restart = new() { Content = L("Restart app"), Style = (Style)Application.Current.Resources["AccentButtonStyle"] };
    private readonly InfoBar failure = Cards.Notice(InfoBarSeverity.Error);
    private readonly Border item = new() { Style = (Style)Application.Current.Resources["UpdateCardStyle"] };
    private readonly TextBlock itemName = new() { TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center };
    private readonly TextBlock itemStatus = new() { Foreground = Cards.Brush("TextFillColorSecondaryBrush"), VerticalAlignment = VerticalAlignment.Center };
    private readonly Button cancel = new() { Content = L("Cancel") };
    private readonly ProgressBar bar = new() { Maximum = 100 };
    private readonly TextBlock details = new() { Style = (Style)Application.Current.Resources["CaptionTextBlockStyle"], Foreground = Cards.Brush("TextFillColorSecondaryBrush"), TextWrapping = TextWrapping.Wrap };
    private readonly Border notes = new() { Style = (Style)Application.Current.Resources["UpdateCardStyle"] };
    private readonly StackPanel whatsNew = new() { Spacing = 4 };
    private readonly StackPanel noteTitles = new() { Spacing = 4 };
    private readonly HyperlinkButton releasePage = new() { Content = L("View on GitHub"), Padding = new Thickness(0), Margin = new Thickness(0, 6, 0, 0) };
    private readonly TextBlock footnote = Cards.Footer(L("The VPN disconnects briefly during the install."));
    private IReadOnlyList<string> shownNotes = [];

    public UpdatePane(AppUi ui, PanePage page)
    {
        this.ui = ui;
        check.Click += (_, _) => ui.Send(new Command.CheckForUpdates());
        updateNow.Click += (_, _) => ui.Send(new Command.UpdateNow());
        restart.Click += (_, _) => ui.Send(new Command.RestartToUpdate());
        cancel.Click += (_, _) => ui.Send(new Command.CancelUpdateDownload());
        appIcon.Loaded += (_, _) => ShowIcon();

        var icons = new Grid { Width = IconSize, Height = IconSize, VerticalAlignment = VerticalAlignment.Center };
        icons.Children.Add(busy);
        icons.Children.Add(upToDate);
        icons.Children.Add(appIcon);
        var text = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        text.Children.Add(headline);
        text.Children.Add(subtitle);
        var actions = new Grid { VerticalAlignment = VerticalAlignment.Center };
        actions.Children.Add(check);
        actions.Children.Add(updateNow);
        actions.Children.Add(restart);
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
        Grid.SetColumn(itemStatus, 1);
        Grid.SetColumn(cancel, 2);
        itemLine.Children.Add(itemName);
        itemLine.Children.Add(itemStatus);
        itemLine.Children.Add(cancel);
        var itemBody = new StackPanel { Spacing = 8 };
        itemBody.Children.Add(itemLine);
        itemBody.Children.Add(bar);
        itemBody.Children.Add(details);
        item.Child = itemBody;

        whatsNew.Children.Add(new TextBlock { Text = L("What's new"), Style = (Style)Application.Current.Resources["BodyStrongTextBlockStyle"], Margin = new Thickness(0, 0, 0, 4) });
        whatsNew.Children.Add(noteTitles);
        var notesBody = new StackPanel();
        notesBody.Children.Add(whatsNew);
        notesBody.Children.Add(releasePage);
        notes.Child = notesBody;

        foreach (var element in new UIElement[] { hero, failure, item, notes, footnote })
        {
            body.Children.Add(element);
        }
        page.BodyPanel.Children.Add(body);
        ui.Send(new Command.UpdatePageOpened());
    }

    public void Refresh()
    {
        body.Visible(ui.AutoUpdate is not null);
        if (ui.AutoUpdate is not { } update)
        {
            return;
        }
        var progress = update.Progress;
        var settled = progress is UpdateProgress.Idle or UpdateProgress.Failed;
        var available = update.Status as UpdateStatus.Available;

        var spinning = settled ? update.Status is UpdateStatus.Checking : progress is UpdateProgress.Installing;
        var current = settled && update.Status is UpdateStatus.UpToDate;
        busy.IsActive = spinning;
        upToDate.Visible(current);
        appIcon.Visible(!spinning && !current);
        headline.Text = update.Headline();
        var line = update.Subtitle();
        subtitle.Visible(line is not null);
        subtitle.Text = line ?? "";
        var action = settled
            ? available is not null ? updateNow : check
            : progress is UpdateProgress.Downloading or UpdateProgress.Preparing ? updateNow : restart;
        check.Apply(action == check ? update.Check : ButtonState.Hidden);
        updateNow.Apply(action == updateNow ? update.UpdateNow : ButtonState.Hidden);
        restart.Apply(action == restart ? update.Restart : ButtonState.Hidden);
        updateNow.Content = progress is UpdateProgress.Failed ? L("Try again") : L("Install now");

        var failedInstall = update.FailedInstallLine();
        var reason = (progress as UpdateProgress.Failed)?.Reason.Message();
        failure.Title = failedInstall ?? "";
        failure.Message = reason ?? "";
        failure.Visible(failedInstall is not null || reason is not null);

        var status = progress.ItemStatus();
        item.Visible(status is not null);
        itemStatus.Text = status ?? "";
        itemName.Text = progress switch
        {
            UpdateProgress.Downloading or UpdateProgress.Preparing => $"FortiVPNManager {available!.Version}",
            UpdateProgress.ReadyToRestart ready => $"FortiVPNManager {ready.Version}",
            UpdateProgress.Installing installing => $"FortiVPNManager {installing.Version}",
            _ => "",
        };
        cancel.Apply(update.Cancel);
        var percent = progress switch
        {
            UpdateProgress.Downloading downloading => downloading.Percent,
            UpdateProgress.Preparing preparing => preparing.Percent,
            _ => (byte?)null,
        };
        bar.Visible(percent is not null);
        bar.Value = percent ?? 0;
        var download = progress as UpdateProgress.Downloading;
        details.Visible(download is not null);
        details.Text = download is null ? "" : $"{Size(download.ReceivedBytes)} / {Size(download.TotalBytes)} · {download.Remaining.Line()}";

        notes.Visible(settled && available is not null);
        footnote.Visible(settled && available is not null);
        if (settled && available is not null)
        {
            releasePage.NavigateUri = new Uri(available.ReleasePageUrl);
            ShowNotes(available.ReleaseNotes() ?? []);
        }
    }

    private void ShowNotes(IReadOnlyList<string> titles)
    {
        whatsNew.Visible(titles.Count > 0);
        if (titles.SequenceEqual(shownNotes))
        {
            return;
        }
        shownNotes = titles;
        noteTitles.Children.Clear();
        foreach (var title in titles)
        {
            var line = new Grid { ColumnSpacing = 8 };
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            var text = new TextBlock { Text = title, TextWrapping = TextWrapping.Wrap };
            Grid.SetColumn(text, 1);
            line.Children.Add(new TextBlock { Text = "•" });
            line.Children.Add(text);
            noteTitles.Children.Add(line);
        }
    }

    private async void ShowIcon()
    {
        var size = (uint)Math.Round(IconSize * appIcon.XamlRoot.RasterizationScale);
        using var stream = await FileRandomAccessStream.OpenAsync(Path.Combine(AppContext.BaseDirectory, "AppIcon.ico"), FileAccessMode.Read);
        var decoder = await BitmapDecoder.CreateAsync(stream);
        var frames = new List<BitmapFrame>();
        for (uint i = 0; i < decoder.FrameCount; i++)
        {
            frames.Add(await decoder.GetFrameAsync(i));
        }
        var frame = frames.Where(f => f.PixelWidth >= size).MinBy(f => f.PixelWidth) ?? frames.MaxBy(f => f.PixelWidth)!;
        var transform = new BitmapTransform { ScaledWidth = size, ScaledHeight = size, InterpolationMode = BitmapInterpolationMode.Fant };
        var bitmap = await frame.GetSoftwareBitmapAsync(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied, transform, ExifOrientationMode.IgnoreExifOrientation, ColorManagementMode.DoNotColorManage);
        var source = new SoftwareBitmapSource();
        await source.SetBitmapAsync(bitmap);
        appIcon.Source = source;
    }

    private static string Size(ulong bytes)
    {
        var buffer = new char[32];
        Marshal.ThrowExceptionForHR(StrFormatByteSizeEx(bytes, RoundToNearestDisplayedDigit, buffer, (uint)buffer.Length));
        return new string(buffer, 0, Array.IndexOf(buffer, '\0'));
    }

    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int StrFormatByteSizeEx(ulong size, int flags, [Out] char[] buffer, uint length);
}
