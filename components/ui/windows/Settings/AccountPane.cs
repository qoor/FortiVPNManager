using CommunityToolkit.WinUI.Controls;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.Windows.Storage.Pickers;
using Windows.System;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Account pane (components/ui/macos/AccountPane.swift): the VPN account and the Gmail account
/// that receives the OTP, saved together with Save at the top (and Ctrl+S). The OTP method and the
/// Google account controls act at once instead of waiting for Save. No certificate fields: gateway
/// certificates are trusted automatically.
/// </summary>
internal sealed class AccountPane : IPane
{
    private const double FieldWidth = 320;

    private readonly AppUi ui;
    private readonly StackPanel body = new() { Spacing = 4 };
    private readonly Button revert = new() { Content = L("Revert") };
    private readonly Button save = new() { Content = L("Save") };
    private readonly InfoBar savedAccountProblem = Cards.Notice(InfoBarSeverity.Error);
    private readonly InfoBar feedback = Cards.Notice(InfoBarSeverity.Error);

    private readonly TextBox host = new() { PlaceholderText = "vpn.example.com" };
    private readonly TextBox port = new() { PlaceholderText = "443" };
    private readonly TextBox username = new();
    private readonly PasswordBox vpnPassword = new();
    private readonly TextBox realm = new() { PlaceholderText = L("Optional") };
    private readonly TextBlock vpnFooter = Cards.Footer(L("Account changes apply from the next connection."));

    private readonly TextBox gmailAddress = new() { PlaceholderText = "name@example.com" };
    private readonly ComboBox method = new();
    private readonly InfoBar methodError = Cards.Notice(InfoBarSeverity.Error);
    private readonly StackPanel gmailApiRows = new() { Spacing = 4 };
    private readonly SettingsCard googleAccount = Cards.Card(L("Google account"));
    private readonly ProgressRing googleProgress = new() { Width = 16, Height = 16, IsActive = true };
    private readonly FontIcon googleMark = new() { FontSize = 14 };
    private readonly TextBlock googleText = new() { VerticalAlignment = VerticalAlignment.Center, TextWrapping = TextWrapping.Wrap, MaxWidth = 240 };
    private readonly Button cancelSignIn = new() { Content = L("Cancel") };
    private readonly Button disconnectGoogle = new() { Content = L("Disconnect") };
    private readonly Button connectGoogle = new() { Content = L("Connect Google Account…") };
    private readonly InfoBar googleError = Cards.Notice(InfoBarSeverity.Error);
    private readonly PasswordBox fallbackPassword = new();
    private readonly SettingsExpander oauthClient = new() { Header = L("OAuth client (advanced)") };
    private readonly TextBlock clientLabel = Cards.Value();
    private readonly FontIcon clientWarning = new() { FontSize = 14, Glyph = "\uE7BA" };
    private readonly Button useBuiltInClient = new() { Content = L("Use Built-in Client") };
    private readonly Button importClient = new() { Content = L("Import JSON…") };
    private readonly InfoBar secretMissing = Cards.Notice(InfoBarSeverity.Warning);
    private readonly SettingsCard imapPasswordCard = Cards.Card(L("App password"));
    private readonly PasswordBox imapPassword = new();
    private readonly Button testGmail = new() { Content = L("Test Gmail Connection") };
    private readonly ProgressRing testProgress = new() { Width = 16, Height = 16, IsActive = true };
    private readonly TextBlock testChecking = new() { Text = L("Checking…"), VerticalAlignment = VerticalAlignment.Center, Foreground = Cards.Brush("TextFillColorSecondaryBrush") };
    private readonly Grid testResult;
    private readonly FontIcon testMark;
    private readonly TextBlock testText;
    private readonly TextBlock methodFooter = Cards.Footer("");

    private bool refreshing;
    private bool oauthClientOpenedForMissing;

    public AccountPane(AppUi ui, PanePage page)
    {
        this.ui = ui;
        revert.Click += (_, _) => ui.Send(new Command.RevertAccount());
        save.Style = (Style)Application.Current.Resources["AccentButtonStyle"];
        save.Click += (_, _) => ui.Send(new Command.SaveAccount());
        ToolTipService.SetToolTip(save, L("Save account settings (Ctrl+S)"));
        save.KeyboardAccelerators.Add(new KeyboardAccelerator { Key = VirtualKey.S, Modifiers = VirtualKeyModifiers.Control });
        save.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        page.ActionsPanel.Children.Add(revert);
        page.ActionsPanel.Children.Add(save);

        body.Children.Add(savedAccountProblem);
        body.Children.Add(feedback);
        AddVpnRows();
        AddGmailRows();
        page.BodyPanel.Children.Add(body);
        (testResult, testMark, testText) = Cards.StatusLine();
        testGmail.Click += (_, _) => ui.Send(new Command.TestGmail());
        body.Children.Add(Cards.ActionCard(Cards.Buttons(testGmail, testProgress, testChecking, testResult)));
        body.Children.Add(methodFooter);
        var filterTip = Cards.Footer(L("Recommended: create a Gmail filter for subject:AuthCode that never sends it to Spam."));
        filterTip.Margin = new Thickness(1, 2, 0, 0);
        body.Children.Add(filterTip);
    }

    private void AddVpnRows()
    {
        body.Children.Add(Cards.SectionHeader(L("VPN"), first: true));
        body.Children.Add(Cards.Card(L("Server address"), Field(host, draft => draft.Host, (draft, text) => draft.Host = text)).Fit(host, FieldWidth));
        body.Children.Add(Cards.Card(L("Port"), Field(port, draft => draft.Port, (draft, text) => draft.Port = text)).Fit(port, 96));
        body.Children.Add(Cards.Card(L("Username"), Field(username, draft => draft.Username, (draft, text) => draft.Username = text)).Fit(username, FieldWidth));
        body.Children.Add(Cards.Card(L("Password"), Field(vpnPassword, draft => draft.VpnPassword, (draft, text) => draft.VpnPassword = text)).Fit(vpnPassword, FieldWidth));
        var realmRow = Remembered(new SettingsExpander { Header = L("Realm (advanced)") }, "realm");
        realmRow.Items.Add(Cards.Card(L("Realm"), Field(realm, draft => draft.Realm, (draft, text) => draft.Realm = text)).Fit(realm, FieldWidth));
        body.Children.Add(realmRow);
        body.Children.Add(vpnFooter);
    }

    private void AddGmailRows()
    {
        body.Children.Add(Cards.SectionHeader(L("2-Step Verification (Gmail)")));
        body.Children.Add(Cards.Card(L("Gmail address"), Field(gmailAddress, draft => draft.GmailAddress, (draft, text) => draft.GmailAddress = text)).Fit(gmailAddress, FieldWidth));
        method.Items.Add(L("Google account (Gmail API)"));
        method.Items.Add(L("App password (IMAP)"));
        method.SelectionChanged += (_, _) =>
        {
            if (!refreshing && method.SelectedIndex >= 0)
            {
                ui.Send(new Command.SetOtpMethod(method.SelectedIndex == 0 ? OtpMethod.GmailApi : OtpMethod.Imap));
            }
        };
        var methodCard = Cards.Card(L("Connection method"), method).Fit(method, FieldWidth);
        ToolTipService.SetToolTip(methodCard, L("Google account is recommended: it sees the OTP email at once. IMAP can be several seconds late."));
        body.Children.Add(methodCard);
        body.Children.Add(methodError);

        // The Gmail API: the Google account, the optional IMAP fallback and the OAuth client.
        cancelSignIn.Click += (_, _) => ui.Send(new Command.CancelGoogleSignIn());
        disconnectGoogle.Click += async (_, _) => await ConfirmGoogleDisconnect();
        connectGoogle.Click += (_, _) => ui.Send(new Command.ConnectGoogleAccount());
        var status = Cards.Row(googleProgress, googleMark, googleText);
        status.Spacing = 6;
        status.VerticalAlignment = VerticalAlignment.Center;
        googleAccount.Content = Cards.Buttons(status, cancelSignIn, disconnectGoogle, connectGoogle);
        gmailApiRows.Children.Add(googleAccount);
        gmailApiRows.Children.Add(googleError);

        var fallback = Remembered(new SettingsExpander { Header = L("IMAP fallback (optional)") }, "imapFallback");
        fallback.Items.Add(new SettingsCard
        {
            Header = L("App password"),
            Description = L("When the Gmail API is unavailable, this app password reads the OTP email."),
            Content = Field(fallbackPassword, draft => draft.AppPassword, (draft, text) => draft.AppPassword = text),
        }.Fit(fallbackPassword, FieldWidth));
        gmailApiRows.Children.Add(fallback);

        useBuiltInClient.Click += (_, _) => ui.Send(new Command.UseBuiltInOAuthClient());
        importClient.Click += async (_, _) => await ImportOAuthClient();
        ToolTipService.SetToolTip(importClient, L("Use a Desktop app OAuth client JSON instead of the built-in client."));
        clientWarning.Foreground = Cards.Brush("SystemFillColorCautionBrush");
        var clientRow = Cards.Row(clientWarning, clientLabel);
        clientRow.Spacing = 6;
        oauthClient.Items.Add(new SettingsCard
        {
            Header = L("OAuth client"),
            Description = L("Changing the client requires connecting the Google account again."),
            Content = clientRow,
        });
        oauthClient.Items.Add(new SettingsCard { Content = Cards.Buttons(useBuiltInClient, importClient) });
        gmailApiRows.Children.Add(Remembered(oauthClient, "oauthClient"));
        secretMissing.Show(null);
        gmailApiRows.Children.Add(secretMissing);
        body.Children.Add(gmailApiRows);

        // IMAP: the app password is required.
        imapPasswordCard.Content = Field(imapPassword, draft => draft.AppPassword, (draft, text) => draft.AppPassword = text);
        imapPasswordCard.Fit(imapPassword, FieldWidth);
        body.Children.Add(imapPasswordCard);
    }

    /// <summary>
    /// A field of the pane: each edit is sent with all the fields as typed. Only a change from the
    /// draft counts: the text a refresh puts in reports its change later, when it equals the draft.
    /// </summary>
    private TextBox Field(TextBox box, Func<AccountDraft, string> read, Action<AccountDraft, string> keep)
    {
        box.TextChanged += (_, _) => Edited(box.Text, read, keep);
        return box;
    }

    private PasswordBox Field(PasswordBox box, Func<AccountDraft, string> read, Action<AccountDraft, string> keep)
    {
        box.PasswordChanged += (_, _) => Edited(box.Password, read, keep);
        return box;
    }

    private void Edited(string text, Func<AccountDraft, string> read, Action<AccountDraft, string> keep)
    {
        if (refreshing || text == read(ui.AccountDraft))
        {
            return;
        }
        keep(ui.AccountDraft, text);
        ui.Send(ui.AccountDraft.Edit());
    }

    /// <summary>A disclosure keeps whether the person opened it while the app runs, as on macOS.</summary>
    private SettingsExpander Remembered(SettingsExpander expander, string key)
    {
        expander.IsExpanded = ui.OpenDisclosures.Contains(key);
        expander.Expanded += (_, _) =>
        {
            if (!refreshing)
            {
                ui.OpenDisclosures.Add(key);
            }
        };
        expander.Collapsed += (_, _) =>
        {
            if (!refreshing)
            {
                ui.OpenDisclosures.Remove(key);
            }
        };
        return expander;
    }

    public void Refresh()
    {
        // The pane and its buttons wait for both sections, as on macOS.
        var ready = ui.Config is not null && ui.Gmail is not null;
        body.Visible(ready);
        save.Visible(ready);
        revert.Visible(ready);
        if (ui.Config is not { } config || ui.Gmail is not { } gmail)
        {
            return;
        }
        refreshing = true;
        var account = config.Account;
        revert.Apply(account.Revert);
        save.Apply(account.Save);
        savedAccountProblem.Show(account.SavedAccountProblem is { } problem ? L("Saved account error: %@", problem.Message()) : null);
        feedback.Severity = account.Feedback is AccountFeedback.Saved ? InfoBarSeverity.Success : InfoBarSeverity.Error;
        feedback.Show(account.Feedback switch
        {
            AccountFeedback.Saved => L("Saved."),
            AccountFeedback.Failed failed => failed.Error.Message(),
            _ => null,
        });

        var draft = ui.AccountDraft;
        Set(host, draft.Host);
        Set(port, draft.Port);
        Set(username, draft.Username);
        Set(realm, draft.Realm);
        Set(gmailAddress, draft.GmailAddress);
        Set(vpnPassword, draft.VpnPassword);
        Set(fallbackPassword, draft.AppPassword);
        Set(imapPassword, draft.AppPassword);
        vpnPassword.PlaceholderText = AccountText.StoredPrompt(account.VpnPasswordStored);
        fallbackPassword.PlaceholderText = account.AppPasswordStored ? AccountText.StoredPrompt(true) : L("Optional");
        imapPassword.PlaceholderText = account.AppPasswordStored ? AccountText.StoredPrompt(true) : L("16-character app password");
        vpnFooter.Visible(ui.Connection?.AccountAppliesNextConnection == true);

        ShowGmail(gmail);
        refreshing = false;
    }

    private void ShowGmail(GmailSection gmail)
    {
        var api = gmail.Method == OtpMethod.GmailApi;
        method.SelectedIndex = api ? 0 : 1;
        methodError.Show(gmail.MethodError?.Message());
        gmailApiRows.Visible(api);
        imapPasswordCard.Visible(!api);
        methodFooter.Text = api
            ? L("Connecting opens Google sign-in in your web browser. Sign in with the Gmail address above.")
            : L("Uses a 16-character app password created in your Google Account. Spaces are ignored.");

        var busy = gmail.GoogleAccount is GoogleAccountStatus.SigningIn or GoogleAccountStatus.Disconnecting;
        googleProgress.Visible(busy);
        googleMark.Visible(gmail.GoogleAccount is GoogleAccountStatus.Connected);
        googleMark.SetSucceeded(true);
        (googleText.Text, googleText.Foreground) = gmail.GoogleAccount switch
        {
            GoogleAccountStatus.SigningIn => (L("Signing in via browser…"), Cards.Brush("TextFillColorSecondaryBrush")),
            GoogleAccountStatus.Disconnecting => (L("Disconnecting"), Cards.Brush("TextFillColorSecondaryBrush")),
            // The address is the Gmail address above; the Gmail test and the OTP session catch a sign-in with another account.
            GoogleAccountStatus.Connected => (L("Connected"), Cards.Brush("TextFillColorPrimaryBrush")),
            GoogleAccountStatus.NotConnected => (L("Not connected"), Cards.Brush("TextFillColorSecondaryBrush")),
            _ => throw new InvalidOperationException($"unknown Google account status {gmail.GoogleAccount}"),
        };
        cancelSignIn.Apply(gmail.CancelSignIn);
        disconnectGoogle.Apply(gmail.DisconnectGoogle);
        connectGoogle.Apply(gmail.ConnectGoogle);
        googleAccount.Describe(gmail.ConnectBlocker?.Message());
        googleError.Show(gmail.GoogleError?.Message(ui.Time));

        // Opened on its own while there is no client, so the import is easy to find; once there is one,
        // it goes back to how the person left it.
        var notConfigured = gmail.OauthClient is OAuthClientSource.NotConfigured;
        if (notConfigured != oauthClientOpenedForMissing)
        {
            oauthClient.IsExpanded = notConfigured || ui.OpenDisclosures.Contains("oauthClient");
        }
        oauthClientOpenedForMissing = notConfigured;
        clientLabel.Text = gmail.OauthClient.Label();
        clientWarning.Visible(notConfigured);
        ToolTipService.SetToolTip(clientLabel, gmail.OauthClient switch
        {
            OAuthClientSource.BuiltIn builtIn => builtIn.ClientId,
            OAuthClientSource.Imported imported => imported.ClientId,
            _ => null,
        });
        useBuiltInClient.Apply(gmail.UseBuiltInClient);
        importClient.Apply(gmail.ImportClient);
        secretMissing.Show(gmail.ImportedClientSecretMissing ? L("The imported client's secret is missing from Credential Manager. Import the JSON again.") : null);

        testGmail.Apply(gmail.TestButton);
        var running = gmail.Test is GmailTest.Running;
        testProgress.Visible(running);
        testChecking.Visible(running);
        var result = gmail.Test.Result();
        testResult.Visible(result is not null);
        if (result is { } shown)
        {
            testText.Text = shown.Text;
            testMark.SetSucceeded(shown.Succeeded);
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
            XamlRoot = googleAccount.XamlRoot,
            Title = L("Disconnect the Google account?"),
            Content = L("To connect again, you'll sign in to Google in your web browser."),
            PrimaryButtonText = L("Disconnect"),
            CloseButtonText = L("Cancel"),
            DefaultButton = ContentDialogButton.Primary,
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            ui.Send(new Command.DisconnectGoogleAccount());
        }
    }

    /// <summary>A file dialog in Downloads, where the browser saves the client JSON; the runtime reads the file.</summary>
    private async Task ImportOAuthClient()
    {
        var picker = new FileOpenPicker(googleAccount.XamlRoot.ContentIslandEnvironment.AppWindowId)
        {
            SuggestedStartLocation = PickerLocationId.Downloads,
            CommitButtonText = L("Import"),
        };
        picker.FileTypeFilter.Add(".json");
        if (await picker.PickSingleFileAsync() is { } file)
        {
            ui.Send(new Command.ImportOAuthClient(file.Path));
        }
    }
}
