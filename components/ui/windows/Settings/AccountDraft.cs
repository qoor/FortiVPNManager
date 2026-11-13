using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Account pane's fields as typed (components/ui/macos/AppUI.swift). The runtime keeps the draft
/// that counts; these fields are replaced only when it replaces what was typed, which it marks with a
/// new revision. Kept by the app, so the typing survives going to another pane.
/// </summary>
public sealed class AccountDraft
{
    private bool loaded;

    public string Host { get; set; } = "";

    public string Port { get; set; } = "";

    public string Username { get; set; } = "";

    public string Realm { get; set; } = "";

    public string GmailAddress { get; set; } = "";

    public string VpnPassword { get; set; } = "";

    public string AppPassword { get; set; } = "";

    public ulong Revision { get; private set; }

    public void Replace(AccountFormState state)
    {
        if (loaded && state.Revision == Revision)
        {
            return;
        }
        loaded = true;
        Revision = state.Revision;
        Host = state.Host;
        Port = state.Port;
        Username = state.Username;
        Realm = state.Realm;
        GmailAddress = state.GmailAddress;
        // A password the runtime still holds stays typed, e.g. when the file changed other fields.
        if (!state.VpnPasswordEntered)
        {
            VpnPassword = "";
        }
        if (!state.AppPasswordEntered)
        {
            AppPassword = "";
        }
    }

    /// <summary>The edit to send after any field changed, against the revision it was typed on.</summary>
    public Command Edit() => new Command.EditAccount(Revision, new AccountForm(Host, Port, Username, Realm, GmailAddress, VpnPassword, AppPassword));
}
