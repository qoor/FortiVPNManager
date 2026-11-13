using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Windows.System;
using static FortiVPNManager.Contract.AppInternal;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Exception Rules pane (components/ui/macos/ExceptionRulesPane.swift). Edits are a draft until
/// Apply, which with Discard Changes sits at the top (and Ctrl+S), like Save in the Account pane.
/// </summary>
internal sealed class ExceptionRulesPane : IPane
{
    private readonly AppUi ui;
    private readonly StackPanel body = new() { Spacing = 4 };
    private readonly Button discard = new() { Content = L("Discard Changes") };
    private readonly Button apply = new() { Content = L("Apply") };
    private readonly TextBlock currentNetwork = Cards.Value();
    private readonly Grid statusLine;
    private readonly FontIcon statusMark;
    private readonly TextBlock statusText;
    private readonly InfoBar applyFailed = Cards.Notice(InfoBarSeverity.Error);
    private readonly InfoBar stoppedRetrying = Cards.Notice(InfoBarSeverity.Error);
    private readonly AddressList networks;
    private readonly AddressList targets;
    private readonly Button addCurrentNetwork = new() { Content = L("Add Current Network") };
    /// <summary>The rows a search result goes to, by the setting's catalog key.</summary>
    private readonly Dictionary<string, FrameworkElement> rows = [];
    private bool editingNetworkRow;
    private bool editingTargetRow;

    public ExceptionRulesPane(AppUi ui, PanePage page)
    {
        this.ui = ui;
        discard.Click += (_, _) =>
        {
            EndEditing();
            ui.Send(new Command.DiscardBypassChanges());
        };
        apply.Style = (Style)Application.Current.Resources["AccentButtonStyle"];
        apply.Click += (_, _) =>
        {
            EndEditing();
            ui.Send(new Command.ApplyBypassRules());
        };
        ToolTipService.SetToolTip(apply, L("Apply exception rules (Ctrl+S)"));
        apply.KeyboardAccelerators.Add(new KeyboardAccelerator { Key = VirtualKey.S, Modifiers = VirtualKeyModifiers.Control });
        apply.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        // Pressing them leaves the focus in a row being typed, as macOS buttons do: the click then ends
        // the edit itself before it applies or discards, and is never lost to the edit ending first.
        discard.AllowFocusOnInteraction = false;
        apply.AllowFocusOnInteraction = false;
        page.ActionsPanel.Children.Add(discard);
        page.ActionsPanel.Children.Add(apply);

        (statusLine, statusMark, statusText) = Cards.StatusLine();
        body.Children.Add(rows["Current network"] = Cards.Card(L("Current network"), currentNetwork));
        body.Children.Add(rows["Status"] = Cards.Card(L("Status"), statusLine));
        body.Children.Add(applyFailed);
        body.Children.Add(stoppedRetrying);

        networks = new AddressList(
            L("Double-click here or click + to add a network."), L("e.g. 10.20.161.0/24"), 44,
            L("Add Network"), L("Add a network"), L("Remove Selected Network"),
            rows => ui.Send(new Command.SetBypassRows(BypassList.Networks, rows)),
            editing => { editingNetworkRow = editing; Refresh(); });
        addCurrentNetwork.Click += (_, _) => ui.Send(new Command.AddCurrentNetwork());
        body.Children.Add(Cards.SectionHeader(L("Networks where the rules apply")));
        body.Children.Add(rows["Networks where the rules apply"] = networks.View);
        rows["Add Current Network"] = addCurrentNetwork;
        var networkButtons = new Grid { Margin = new Thickness(0, 4, 0, 0) };
        networkButtons.Children.Add(Cards.Row(networks.Add, networks.Remove));
        addCurrentNetwork.HorizontalAlignment = HorizontalAlignment.Right;
        networkButtons.Children.Add(addCurrentNetwork);
        body.Children.Add(networkButtons);

        // A PC is used on a few networks, unlike the many exception targets.
        targets = new AddressList(
            L("Double-click here or click + to add a host or network."), L("e.g. 10.30.161.16 or 10.30.170.0/24"), 175,
            L("Add Exception Target"), L("Add a host or network"), L("Remove Selected Exception Target"),
            rows => ui.Send(new Command.SetBypassRows(BypassList.Targets, rows)),
            editing => { editingTargetRow = editing; Refresh(); });
        body.Children.Add(Cards.SectionHeader(L("Exception targets")));
        body.Children.Add(rows["Exception targets"] = targets.View);
        var targetButtons = Cards.Row(targets.Add, targets.Remove);
        targetButtons.Margin = new Thickness(0, 4, 0, 0);
        body.Children.Add(targetButtons);
        var footer = Cards.Footer(L("While this PC is on one of the networks above, the exception targets skip the VPN and go out directly through that network's router."));
        ToolTipService.SetToolTip(footer, L("Elsewhere, everything goes through the VPN. Routes are saved in Windows network settings and survive restarts."));
        body.Children.Add(footer);
        page.BodyPanel.Children.Add(body);
    }

    public FrameworkElement? OpenRow(string key) => rows.GetValueOrDefault(key);

    public void Refresh()
    {
        body.Visible(ui.BypassRules is not null);
        discard.Visible(ui.BypassRules is not null);
        apply.Visible(ui.BypassRules is not null);
        if (ui.BypassRules is not { } rules)
        {
            return;
        }
        // A row being typed reaches the runtime when its edit ends, which Apply and Discard do first.
        var editing = editingNetworkRow || editingTargetRow;
        discard.Apply(WhileEditing(rules.Discard, editing));
        apply.Apply(WhileEditing(rules.Apply, editing));
        currentNetwork.Text = rules.CurrentNetworkText();

        // Marked addresses and unapplied changes come first; otherwise how the rules stand on this network.
        if (rules.Networks.ProblemRows.Count > 0 || rules.Targets.ProblemRows.Count > 0)
        {
            statusText.Text = L("Fix the marked addresses to apply.");
            statusMark.SetStyle(NoticeStyle.Error);
        }
        else if (rules.HasChanges)
        {
            statusText.Text = L("There are changes that haven't been applied.");
            statusMark.Glyph = "\uE70F";
            statusMark.Foreground = Cards.Brush("TextFillColorSecondaryBrush");
        }
        else
        {
            var (text, style) = rules.Status.Line(ui.Time);
            statusText.Text = text;
            statusMark.SetStyle(style);
        }
        applyFailed.Show(rules.Notice is BypassNotice.ApplyFailed failed ? failed.Error.Message() : null);
        stoppedRetrying.Show(rules.StoppedRetryingMessage());

        networks.Show(rules.Networks.Rows, ProblemRows(rules.Networks), RowMessages(rules, BypassList.Networks), rules.Networks.AddRow == ButtonState.Enabled);
        targets.Show(rules.Targets.Rows, ProblemRows(rules.Targets), RowMessages(rules, BypassList.Targets), rules.Targets.AddRow == ButtonState.Enabled);
        addCurrentNetwork.Apply(rules.AddCurrentNetwork);
    }

    public void FinishTyping() => EndEditing();

    private void EndEditing()
    {
        networks.CommitEdit();
        targets.CommitEdit();
    }

    /// <summary>While a row is typed, a disabled Apply or Discard is offered, as the edit may change the list.</summary>
    private static ButtonState WhileEditing(ButtonState state, bool editing) =>
        editing && state == ButtonState.Disabled ? ButtonState.Enabled : state;

    private static HashSet<int> ProblemRows(RuleList list) => list.ProblemRows.Select(row => (int)row).ToHashSet();

    /// <summary>
    /// The problem the runtime explains, beside its row in <paramref name="list"/>. It reports only the
    /// first problem of the networks, or else of the targets; the other marked rows show the mark alone.
    /// </summary>
    private static Dictionary<int, (string Text, string Help)> RowMessages(BypassRulesSection rules, BypassList list)
    {
        var explained = rules.Networks.ProblemRows.Count == 0 ? BypassList.Targets : BypassList.Networks;
        var rows = list == BypassList.Networks ? rules.Networks : rules.Targets;
        if (list != explained || rules.Notice is not BypassNotice.RowProblem notice || rows.ProblemRows.Count == 0)
        {
            return [];
        }
        var index = (int)rows.ProblemRows[0];
        if (index >= rows.Rows.Count || rows.Rows[index] != notice.Row)
        {
            return [];
        }
        return new() { [index] = (notice.Problem.RowMessage(), notice.Problem.Message(notice.Row)) };
    }
}
