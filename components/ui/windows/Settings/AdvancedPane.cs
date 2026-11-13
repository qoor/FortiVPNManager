using CommunityToolkit.WinUI.Controls;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Advanced pane (components/ui/macos/AdvancedPane.swift): the Helper, the VPN client it runs,
/// and the settings file. Windows has no approval step for the Helper; the install script registers it.
/// </summary>
internal sealed class AdvancedPane : IPane
{
    private readonly AppUi ui;
    private readonly StackPanel body = new() { Spacing = 4 };
    private readonly Grid helperLine;
    private readonly FontIcon helperMark;
    private readonly TextBlock helperText;
    private readonly Button registerHelper = new() { Content = L("Register Helper") };
    private readonly SettingsCard registerCard;
    private readonly InfoBar registerError = Cards.Notice(InfoBarSeverity.Error);
    private readonly StackPanel client = new() { Spacing = 4 };
    private readonly TextBlock clientVersion = Cards.Value();
    private readonly Button showClient = new() { Content = L("Show in File Explorer") };
    private readonly TextBlock clientNotFound = Cards.Value();
    private readonly InfoBar untested = Cards.Notice(InfoBarSeverity.Warning);
    private readonly InfoBar clientProblem = Cards.Notice(InfoBarSeverity.Error);
    private readonly TextBlock filePath = new() { IsTextSelectionEnabled = true, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center };
    private readonly Button showFile = new() { Content = L("Show in File Explorer") };
    private readonly SettingsCard fileCard;
    private readonly InfoBar loadError = Cards.Notice(InfoBarSeverity.Error);
    private readonly StackPanel warnings = new() { Spacing = 4 };
    private string? clientPath;
    private string? settingsPath;
    private IReadOnlyList<ConfigWarning> shownWarnings = [];

    public AdvancedPane(AppUi ui, PanePage page)
    {
        this.ui = ui;
        (helperLine, helperMark, helperText) = Cards.StatusLine();
        registerHelper.Click += (_, _) => ui.Send(new Command.RegisterHelper());
        body.Children.Add(Cards.SectionHeader(L("Helper"), first: true));
        body.Children.Add(Cards.Card(L("Status"), helperLine));
        registerCard = Cards.ActionCard(registerHelper);
        body.Children.Add(registerCard);
        body.Children.Add(registerError);

        // The path runs long and would wrap; the button shows it in File Explorer, its tooltip as text.
        showClient.Click += (_, _) =>
        {
            if (clientPath is { } path)
            {
                Cards.ShowInFileExplorer(path);
            }
        };
        clientNotFound.Text = L("Not found");
        client.Children.Add(Cards.SectionHeader(L("VPN client")));
        client.Children.Add(Cards.Card(L("Type"), new TextBlock { Text = "openconnect", Style = (Style)Application.Current.Resources["ValueStyle"] }));
        client.Children.Add(Cards.Card(L("Version"), clientVersion));
        client.Children.Add(Cards.Card(L("Path"), Cards.Row(showClient, clientNotFound)));
        client.Children.Add(untested);
        client.Children.Add(clientProblem);
        body.Children.Add(client);

        showFile.Click += (_, _) =>
        {
            if (settingsPath is { } path)
            {
                Cards.ShowInFileExplorer(path);
            }
        };
        fileCard = new SettingsCard { Header = filePath, Content = showFile };
        body.Children.Add(Cards.SectionHeader(L("Settings File")));
        body.Children.Add(fileCard);
        body.Children.Add(loadError);
        body.Children.Add(warnings);
        page.BodyPanel.Children.Add(body);
    }

    public void Refresh()
    {
        body.Visible(ui.Readiness is not null && ui.Config is not null);
        if (ui.Readiness is not { } readiness || ui.Config is not { } config)
        {
            return;
        }
        helperText.Text = readiness.Helper.Label();
        helperMark.SetStyle(readiness.Helper.Style());
        registerHelper.Apply(readiness.RegisterHelper);
        registerCard.Visible(readiness.RegisterHelper != ButtonState.Hidden);
        registerError.Show(readiness.RegisterError?.HelperMessage());

        client.Visible(readiness.Client is not null);
        if (readiness.Client is { } info)
        {
            clientVersion.Text = info.Version ?? L("Unknown");
            clientPath = info.Path;
            showClient.Visible(info.Path is not null);
            ToolTipService.SetToolTip(showClient, info.Path);
            clientNotFound.Visible(info.Path is null);
            untested.Show(info.UntestedVersion?.Message());
            clientProblem.Show(info.Problem?.Message());
        }

        var file = config.File;
        settingsPath = file.Path;
        filePath.Text = file.Path;
        ToolTipService.SetToolTip(filePath, file.Path);
        showFile.IsEnabled = file.Exists;
        fileCard.Describe(file.Exists ? null : L("Not created yet. Saving the account creates it."));
        loadError.Show(file.LoadError is { } error ? L("%@ Using the last valid settings.", error.Message()) : null);
        if (!file.Warnings.SequenceEqual(shownWarnings))
        {
            shownWarnings = file.Warnings;
            warnings.Children.Clear();
            foreach (var warning in file.Warnings)
            {
                var notice = Cards.Notice(InfoBarSeverity.Warning);
                notice.Show(warning.DisplayMessage());
                warnings.Children.Add(notice);
            }
        }
    }
}
