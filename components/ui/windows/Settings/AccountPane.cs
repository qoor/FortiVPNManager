using CommunityToolkit.WinUI.Controls;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.Windows.Storage.Pickers;
using Windows.System;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Account pane (components/ui/macos/AccountPane.swift): the VPN account and the Gmail account
/// that receives the OTP, saved when a field is committed (Enter, leaving the field, or leaving the
/// pane). The OTP method and the Google account controls act at once. No certificate fields: gateway
/// certificates are trusted automatically.
/// </summary>
internal sealed class AccountPane : IPane
{
    private const double FieldWidth = 320;

    private readonly AppUi _ui;
    private readonly StackPanel _body = new() { Spacing = 4 };
    private readonly InfoBar _savedAccountProblem = Cards.Notice(InfoBarSeverity.Error);
    private readonly InfoBar _saveError = Cards.Notice(InfoBarSeverity.Error);

    private readonly TextBox _server = new() { PlaceholderText = "vpn.example.com" };
    private readonly TextBox _username = new();
    private readonly PasswordBox _vpnPassword = new();
    private readonly TextBox _realm = new() { PlaceholderText = L("Optional") };
    private readonly TextBlock _vpnFooter = Cards.Footer(L("Account changes apply from the next connection."));

    private readonly TextBox _gmailAddress = new() { PlaceholderText = "name@example.com" };
    private readonly ComboBox _method = new();
    private readonly InfoBar _methodError = Cards.Notice(InfoBarSeverity.Error);
    private readonly StackPanel _gmailApiRows = new() { Spacing = 4 };
    private readonly SettingsCard _googleAccount = Cards.Card(L("Google account"));
    private readonly ProgressRing _googleProgress = new() { Width = 16, Height = 16, IsActive = true };
    private readonly FontIcon _googleMark = new() { FontSize = 14 };
    private readonly TextBlock _googleText = new() { VerticalAlignment = VerticalAlignment.Center, TextWrapping = TextWrapping.Wrap, MaxWidth = 240 };
    private readonly Button _cancelSignIn = new() { Content = L("Cancel") };
    private readonly Button _disconnectGoogle = new() { Content = L("Disconnect") };
    private readonly Button _connectGoogle = new() { Content = L("Connect Google Account…") };
    private readonly InfoBar _googleError = Cards.Notice(InfoBarSeverity.Error);
    private readonly PasswordBox _fallbackPassword = new();
    private readonly SettingsExpander _oauthClient = new() { Header = L("OAuth client (advanced)") };
    private readonly TextBlock _clientLabel = Cards.Value();
    private readonly FontIcon _clientWarning = new() { FontSize = 14, Glyph = "\uE7BA" };
    private readonly Button _useBuiltInClient = new() { Content = L("Use Built-in Client") };
    private readonly Button _importClient = new() { Content = L("Import JSON…") };
    private readonly InfoBar _secretMissing = Cards.Notice(InfoBarSeverity.Warning);
    private readonly SettingsCard _imapPasswordCard = Cards.Card(L("App password"));
    private readonly PasswordBox _imapPassword = new();
    private readonly Button _testGmail = new() { Content = L("Test Gmail Connection") };
    private readonly ProgressRing _testProgress = new() { Width = 16, Height = 16, IsActive = true };
    private readonly TextBlock _testChecking = new() { Text = L("Checking…"), VerticalAlignment = VerticalAlignment.Center, Foreground = Cards.Brush("TextFillColorSecondaryBrush") };
    private readonly Grid _testResult;
    private readonly FontIcon _testMark;
    private readonly TextBlock _testText;
    private readonly TextBlock _methodFooter = Cards.Footer("");

    /// <summary>The rows a search result goes to, by the setting's catalog key.</summary>
    private readonly Dictionary<string, FrameworkElement> _rows = [];
    /// <summary>The expander each row in one is in; an expander's own row maps to itself.</summary>
    private readonly Dictionary<FrameworkElement, SettingsExpander> _expanders = [];
    private SettingsCard? _fallbackPasswordCard;
    private bool _refreshing;
    private bool _oauthClientOpenedForMissing;

    public AccountPane(AppUi ui, PanePage page)
    {
        this._ui = ui;
        _body.Children.Add(_savedAccountProblem);
        _body.Children.Add(_saveError);
        AddVpnRows();
        AddGmailRows();
        page.BodyPanel.Children.Add(_body);
        (_testResult, _testMark, _testText) = Cards.StatusLine();
        _testGmail.Click += (_, _) => ui.Send(new Command.TestGmail());
        _body.Children.Add(_rows["Test Gmail Connection"] = Cards.ActionCard(Cards.Buttons(_testGmail, _testProgress, _testChecking, _testResult)));
        _body.Children.Add(_methodFooter);
        TextBlock filterTip = Cards.Footer(L("Recommended: create a Gmail filter for subject:AuthCode that never sends it to Spam."));
        filterTip.Margin = new Thickness(1, 2, 0, 0);
        _body.Children.Add(filterTip);
    }

    private void AddVpnRows()
    {
        _body.Children.Add(Cards.SectionHeader(L("VPN"), first: true));
        _body.Children.Add(_rows["Server address"] = Cards.Card(L("Server address"), Field(_server, draft => draft.Server, (draft, text) => draft.Server = text)).Fit(_server, FieldWidth));
        _body.Children.Add(_rows["Username"] = Cards.Card(L("Username"), Field(_username, draft => draft.Username, (draft, text) => draft.Username = text)).Fit(_username, FieldWidth));
        _body.Children.Add(_rows["Password"] = Cards.Card(L("Password"), Field(_vpnPassword, draft => draft.VpnPassword, (draft, text) => draft.VpnPassword = text)).Fit(_vpnPassword, FieldWidth));
        SettingsExpander realmRow = Remembered(new SettingsExpander { Header = L("Realm (advanced)") }, "realm");
        realmRow.Items.Add(_rows["Realm"] = Cards.Card(L("Realm"), Field(_realm, draft => draft.Realm, (draft, text) => draft.Realm = text)).Fit(_realm, FieldWidth));
        _expanders[_rows["Realm"]] = realmRow;
        _body.Children.Add(realmRow);
        _body.Children.Add(_vpnFooter);
    }

    private void AddGmailRows()
    {
        _body.Children.Add(Cards.SectionHeader(L("2-Step Verification (Gmail)")));
        _body.Children.Add(_rows["Gmail address"] = Cards.Card(L("Gmail address"), Field(_gmailAddress, draft => draft.GmailAddress, (draft, text) => draft.GmailAddress = text)).Fit(_gmailAddress, FieldWidth));
        _method.Items.Add(L("Google account (Gmail API)"));
        _method.Items.Add(L("App password (IMAP)"));
        _method.SelectionChanged += (_, _) =>
        {
            if (!_refreshing && _method.SelectedIndex >= 0)
            {
                _ui.Send(new Command.SetOtpMethod(_method.SelectedIndex == 0 ? OtpMethod.GmailApi : OtpMethod.Imap));
            }
        };
        SettingsCard methodCard = Cards.Card(L("Connection method"), _method).Fit(_method, FieldWidth);
        ToolTipService.SetToolTip(methodCard, L("Google account is recommended: it sees the OTP email at once. IMAP can be several seconds late."));
        _body.Children.Add(_rows["Connection method"] = methodCard);
        _body.Children.Add(_methodError);

        // The Gmail API: the Google account, the optional IMAP fallback and the OAuth client.
        _cancelSignIn.Click += (_, _) => _ui.Send(new Command.CancelGoogleSignIn());
        _disconnectGoogle.Click += async (_, _) => await ConfirmGoogleDisconnect();
        _connectGoogle.Click += (_, _) => _ui.Send(new Command.ConnectGoogleAccount());
        StackPanel status = Cards.Row(_googleProgress, _googleMark, _googleText);
        status.Spacing = 6;
        status.VerticalAlignment = VerticalAlignment.Center;
        _googleAccount.Content = Cards.Buttons(status, _cancelSignIn, _disconnectGoogle, _connectGoogle);
        _gmailApiRows.Children.Add(_rows["Google account"] = _googleAccount);
        _gmailApiRows.Children.Add(_googleError);

        SettingsExpander fallback = Remembered(new SettingsExpander { Header = L("IMAP fallback (optional)") }, "imapFallback");
        _fallbackPasswordCard = new SettingsCard
        {
            Header = L("App password"),
            Description = L("When the Gmail API is unavailable, this app password reads the OTP email."),
            Content = Field(_fallbackPassword, draft => draft.AppPassword, (draft, text) => draft.AppPassword = text),
        }.Fit(_fallbackPassword, FieldWidth);
        fallback.Items.Add(_fallbackPasswordCard);
        _expanders[_fallbackPasswordCard] = fallback;
        _expanders[fallback] = fallback;
        _gmailApiRows.Children.Add(_rows["IMAP fallback (optional)"] = fallback);

        _useBuiltInClient.Click += (_, _) => _ui.Send(new Command.UseBuiltInOAuthClient());
        _importClient.Click += async (_, _) => await ImportOAuthClient();
        ToolTipService.SetToolTip(_importClient, L("Use a Desktop app OAuth client JSON instead of the built-in client."));
        _clientWarning.Foreground = Cards.Brush("SystemFillColorCautionBrush");
        StackPanel clientRow = Cards.Row(_clientWarning, _clientLabel);
        clientRow.Spacing = 6;
        _oauthClient.Items.Add(_rows["OAuth client"] = new SettingsCard
        {
            Header = L("OAuth client"),
            Description = L("Changing the client requires connecting the Google account again."),
            Content = clientRow,
        });
        _oauthClient.Items.Add(_rows[SettingsSearch.OAuthClientButtons] = new SettingsCard { Content = Cards.Buttons(_useBuiltInClient, _importClient) });
        _gmailApiRows.Children.Add(Remembered(_oauthClient, "oauthClient"));
        _expanders[_rows["OAuth client"]] = _oauthClient;
        _expanders[_rows[SettingsSearch.OAuthClientButtons]] = _oauthClient;
        _secretMissing.Show(null);
        _gmailApiRows.Children.Add(_secretMissing);
        _body.Children.Add(_gmailApiRows);

        // IMAP: the app password is required.
        _imapPasswordCard.Content = Field(_imapPassword, draft => draft.AppPassword, (draft, text) => draft.AppPassword = text);
        _imapPasswordCard.Fit(_imapPassword, FieldWidth);
        _body.Children.Add(_imapPasswordCard);
    }

    /// <summary>
    /// A field of the pane: each edit is sent with all the fields as typed, and Enter or leaving the
    /// field saves them. Only a change from the draft counts: the text a refresh puts in reports its
    /// change later, when it equals the draft.
    /// </summary>
    private TextBox Field(TextBox box, Func<AccountDraft, string> read, Action<AccountDraft, string> keep)
    {
        box.TextChanged += (_, _) => Edited(box.Text, read, keep);
        SavesOnCommit(box);
        return box;
    }

    private PasswordBox Field(PasswordBox box, Func<AccountDraft, string> read, Action<AccountDraft, string> keep)
    {
        box.PasswordChanged += (_, _) => Edited(box.Password, read, keep);
        SavesOnCommit(box);
        return box;
    }

    private void SavesOnCommit(Control box)
    {
        box.LostFocus += (_, _) => _ui.Send(new Command.SaveAccount());
        box.KeyDown += (_, args) =>
        {
            if (args.Key == VirtualKey.Enter)
            {
                _ui.Send(new Command.SaveAccount());
            }
        };
    }

    public void FinishTyping() => _ui.Send(new Command.SaveAccount());

    private void Edited(string text, Func<AccountDraft, string> read, Action<AccountDraft, string> keep)
    {
        if (_refreshing || text == read(_ui.AccountDraft))
        {
            return;
        }
        keep(_ui.AccountDraft, text);
        _ui.Send(_ui.AccountDraft.Edit());
    }

    /// <summary>A disclosure keeps whether the person opened it while the app runs, as on macOS.</summary>
    private SettingsExpander Remembered(SettingsExpander expander, string key)
    {
        expander.IsExpanded = _ui.OpenDisclosures.Contains(key);
        expander.Expanded += (_, _) =>
        {
            if (!_refreshing)
            {
                _ui.OpenDisclosures.Add(key);
            }
        };
        expander.Collapsed += (_, _) =>
        {
            if (!_refreshing)
            {
                _ui.OpenDisclosures.Remove(key);
            }
        };
        return expander;
    }

    /// <summary>The app password's row is the fallback's under the Gmail API and a row of its own with IMAP.</summary>
    public FrameworkElement? OpenRow(string key)
    {
        FrameworkElement? row = key == "App password"
            ? _ui.Gmail?.Method == OtpMethod.GmailApi ? _fallbackPasswordCard : _imapPasswordCard
            : _rows.GetValueOrDefault(key);
        if (row is not null && _expanders.TryGetValue(row, out SettingsExpander? expander))
        {
            expander.IsExpanded = true;
        }
        return row;
    }

    public void Refresh()
    {
        // The pane and its buttons wait for both sections, as on macOS.
        bool ready = _ui.Config is not null && _ui.Gmail is not null;
        _body.Visible(ready);
        if (_ui.Config is not { } config || _ui.Gmail is not { } gmail)
        {
            return;
        }
        _refreshing = true;
        AccountFormState account = config.Account;
        _savedAccountProblem.Show(account.SavedAccountProblem is { } problem ? L("Saved account error: %@", problem.Message()) : null);
        _saveError.Show(account.SaveError?.Message());

        AccountDraft draft = _ui.AccountDraft;
        Set(_server, draft.Server);
        Set(_username, draft.Username);
        Set(_realm, draft.Realm);
        Set(_gmailAddress, draft.GmailAddress);
        Set(_vpnPassword, draft.VpnPassword);
        Set(_fallbackPassword, draft.AppPassword);
        Set(_imapPassword, draft.AppPassword);
        _vpnPassword.PlaceholderText = AccountText.StoredPrompt(account.VpnPasswordStored);
        _fallbackPassword.PlaceholderText = account.AppPasswordStored ? AccountText.StoredPrompt(true) : L("Optional");
        _imapPassword.PlaceholderText = account.AppPasswordStored ? AccountText.StoredPrompt(true) : L("16-character app password");
        _vpnFooter.Visible(_ui.Connection?.AccountAppliesNextConnection == true);

        ShowGmail(gmail);
        _refreshing = false;
    }

    private void ShowGmail(GmailSection gmail)
    {
        bool api = gmail.Method == OtpMethod.GmailApi;
        _method.SelectedIndex = api ? 0 : 1;
        _methodError.Show(gmail.MethodError?.Message());
        _gmailApiRows.Visible(api);
        _imapPasswordCard.Visible(!api);
        _methodFooter.Text = api
            ? L("Connecting opens Google sign-in in your web browser. Sign in with the Gmail address above.")
            : L("Uses a 16-character app password created in your Google Account. Spaces are ignored.");

        bool busy = gmail.GoogleAccount is GoogleAccountStatus.SigningIn or GoogleAccountStatus.Disconnecting;
        _googleProgress.Visible(busy);
        _googleMark.Visible(gmail.GoogleAccount is GoogleAccountStatus.Connected);
        _googleMark.SetSucceeded(true);
        (_googleText.Text, _googleText.Foreground) = gmail.GoogleAccount switch
        {
            GoogleAccountStatus.SigningIn => (L("Signing in via browser…"), Cards.Brush("TextFillColorSecondaryBrush")),
            GoogleAccountStatus.Disconnecting => (L("Disconnecting"), Cards.Brush("TextFillColorSecondaryBrush")),
            // The address is the Gmail address above; the Gmail test and the OTP session catch a sign-in with another account.
            GoogleAccountStatus.Connected => (L("Connected"), Cards.Brush("TextFillColorPrimaryBrush")),
            GoogleAccountStatus.NotConnected => (L("Not connected"), Cards.Brush("TextFillColorSecondaryBrush")),
            _ => throw new InvalidOperationException($"unknown Google account status {gmail.GoogleAccount}"),
        };
        _cancelSignIn.Apply(gmail.CancelSignIn);
        _disconnectGoogle.Apply(gmail.DisconnectGoogle);
        _connectGoogle.Apply(gmail.ConnectGoogle);
        _googleAccount.Describe(gmail.ConnectBlocker?.Message());
        _googleError.Show(gmail.GoogleError?.Message(_ui.Time));

        // Opened on its own while there is no client, so the import is easy to find; once there is one,
        // it goes back to how the person left it.
        bool notConfigured = gmail.OauthClient is OAuthClientSource.NotConfigured;
        if (notConfigured != _oauthClientOpenedForMissing)
        {
            _oauthClient.IsExpanded = notConfigured || _ui.OpenDisclosures.Contains("oauthClient");
        }
        _oauthClientOpenedForMissing = notConfigured;
        _clientLabel.Text = gmail.OauthClient.Label();
        _clientWarning.Visible(notConfigured);
        ToolTipService.SetToolTip(_clientLabel, gmail.OauthClient switch
        {
            OAuthClientSource.BuiltIn builtIn => builtIn.ClientId,
            OAuthClientSource.Imported imported => imported.ClientId,
            _ => null,
        });
        _useBuiltInClient.Apply(gmail.UseBuiltInClient);
        _importClient.Apply(gmail.ImportClient);
        _secretMissing.Show(gmail.ImportedClientSecretMissing ? L("The imported client's secret is missing from Credential Manager. Import the JSON again.") : null);

        _testGmail.Apply(gmail.TestButton);
        bool running = gmail.Test is GmailTest.Running;
        _testProgress.Visible(running);
        _testChecking.Visible(running);
        (string Text, bool Succeeded)? result = gmail.Test.Result();
        _testResult.Visible(result is not null);
        if (result is { } shown)
        {
            _testText.Text = shown.Text;
            _testMark.SetSucceeded(shown.Succeeded);
        }
    }

    /// <summary>Changes a field only when the runtime replaced what was typed, so typing keeps its place.</summary>
    private static void Set(TextBox box, string text)
    {
        if (box.Text != text)
        {
            box.Text = text;
        }
    }

    private static void Set(PasswordBox box, string text)
    {
        if (box.Password != text)
        {
            box.Password = text;
        }
    }

    /// <summary>Undoing it takes a new sign-in in the browser, so it is asked first.</summary>
    private async Task ConfirmGoogleDisconnect()
    {
        var dialog = new ContentDialog
        {
            XamlRoot = _googleAccount.XamlRoot,
            Title = L("Disconnect the Google account?"),
            Content = L("To connect again, you'll sign in to Google in your web browser."),
            PrimaryButtonText = L("Disconnect"),
            CloseButtonText = L("Cancel"),
            DefaultButton = ContentDialogButton.Primary,
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            _ui.Send(new Command.DisconnectGoogleAccount());
        }
    }

    /// <summary>A file dialog in Downloads, where the browser saves the client JSON; the runtime reads the file.</summary>
    private async Task ImportOAuthClient()
    {
        var picker = new FileOpenPicker(_googleAccount.XamlRoot.ContentIslandEnvironment.AppWindowId)
        {
            SuggestedStartLocation = PickerLocationId.Downloads,
            CommitButtonText = L("Import"),
        };
        picker.FileTypeFilter.Add(".json");
        if (await picker.PickSingleFileAsync() is { } file)
        {
            _ui.Send(new Command.ImportOAuthClient(file.Path));
        }
    }
}
