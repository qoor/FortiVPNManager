using CommunityToolkit.WinUI.Controls;
using FortiVPNManager.Contract.AppInternal;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Advanced pane (components/ui/macos/AdvancedPane.swift): the Helper, the VPN client it runs,
/// and the settings file. Windows has no approval step for the Helper; the installer registers it.
/// </summary>
internal sealed class AdvancedPane : IPane
{
    private readonly AppUi _ui;
    private readonly StackPanel _body = new() { Spacing = 4 };
    private readonly Grid _helperLine;
    private readonly FontIcon _helperMark;
    private readonly TextBlock _helperText;
    private readonly Button _registerHelper = new() { Content = L("Register Helper") };
    private readonly SettingsCard _registerCard;
    private readonly InfoBar _registerError = Cards.Notice(InfoBarSeverity.Error);
    private readonly StackPanel _backend = new() { Spacing = 4 };
    private readonly TextBlock _backendVersion = Cards.Value();
    private readonly TextBlock _filePath = new() { IsTextSelectionEnabled = true, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center };
    private readonly Button _showFile = new() { Content = L("Show in File Explorer") };
    private readonly SettingsCard _fileCard;
    private readonly InfoBar _loadError = Cards.Notice(InfoBarSeverity.Error);
    private readonly StackPanel _warnings = new() { Spacing = 4 };
    /// <summary>The rows a search result goes to, by the setting's catalog key.</summary>
    private readonly Dictionary<string, FrameworkElement> _rows = [];
    private string? _settingsPath;
    private IReadOnlyList<ConfigWarning> _shownWarnings = [];

    public AdvancedPane(AppUi ui, PanePage page)
    {
        this._ui = ui;
        (_helperLine, _helperMark, _helperText) = Cards.StatusLine();
        _registerHelper.Click += (_, _) => ui.Send(new CommandRegisterHelper());
        _body.Children.Add(Cards.SectionHeader(L("Helper"), first: true));
        _body.Children.Add(_rows["Helper"] = Cards.Card(L("Status"), _helperLine));
        _registerCard = Cards.ActionCard(_registerHelper);
        _body.Children.Add(_rows["Register Helper"] = _registerCard);
        _body.Children.Add(_registerError);

        _backend.Children.Add(Cards.SectionHeader(L("VPN client")));
        _backend.Children.Add(_rows["Type"] = Cards.Card(L("Type"), new TextBlock { Text = "openconnect", Style = (Style)Application.Current.Resources["ValueStyle"] }));
        _backend.Children.Add(_rows["Version"] = Cards.Card(L("Version"), _backendVersion));
        _body.Children.Add(_backend);

        _showFile.Click += (_, _) =>
        {
            if (_settingsPath is { } path)
            {
                Cards.ShowInFileExplorer(path);
            }
        };
        _fileCard = new SettingsCard { Header = _filePath, Content = _showFile };
        _body.Children.Add(Cards.SectionHeader(L("Settings File")));
        _body.Children.Add(_rows["Settings File"] = _fileCard);
        _body.Children.Add(_loadError);
        _body.Children.Add(_warnings);
        page.BodyPanel.Children.Add(_body);
    }

    public FrameworkElement? OpenRow(string key) => _rows.GetValueOrDefault(key);

    public void Refresh()
    {
        _body.Visible(_ui.Readiness is not null && _ui.Config is not null);
        if (_ui.Readiness is not { } readiness || _ui.Config is not { } config)
        {
            return;
        }
        _helperText.Text = readiness.Helper.Label();
        _helperMark.SetStyle(readiness.Helper.Style());
        _registerHelper.Apply(readiness.RegisterHelper);
        _registerCard.Visible(readiness.RegisterHelper != ButtonState.Hidden);
        _registerError.Show(readiness.RegisterError?.HelperMessage());

        _backend.Visible(readiness.Backend is not null);
        if (readiness.Backend is { } info)
        {
            _backendVersion.Text = info.Version ?? L("Unknown");
        }

        ConfigFileState file = config.File;
        _settingsPath = file.Path;
        _filePath.Text = file.Path;
        ToolTipService.SetToolTip(_filePath, file.Path);
        _showFile.IsEnabled = file.Exists;
        _fileCard.Describe(file.Exists ? null : L("Not created yet. Saving the account creates it."));
        _loadError.Show(file.LoadError is { } error ? L("%@ Using the last valid settings.", error.Message()) : null);
        if (!file.Warnings.SequenceEqual(_shownWarnings))
        {
            _shownWarnings = file.Warnings;
            _warnings.Children.Clear();
            foreach (ConfigWarning warning in file.Warnings)
            {
                InfoBar notice = Cards.Notice(InfoBarSeverity.Warning);
                notice.Show(warning.DisplayMessage());
                _warnings.Children.Add(notice);
            }
        }
    }
}
