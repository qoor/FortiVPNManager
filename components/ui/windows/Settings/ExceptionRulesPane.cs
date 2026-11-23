using FortiVPNManager.Contract.AppInternal;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using static FortiVPNManager.Ui.Text.Strings;

namespace FortiVPNManager.Ui.Settings;

/// <summary>
/// The Exception Rules pane (components/ui/macos/ExceptionRulesPane.swift). Every finished edit is
/// sent; the runtime saves the lists while no row has a problem.
/// </summary>
internal sealed class ExceptionRulesPane : IPane
{
    private readonly AppUi _ui;
    private readonly StackPanel _body = new() { Spacing = 4 };
    private readonly TextBlock _currentNetwork = Cards.Value();
    private readonly Grid _statusLine;
    private readonly FontIcon _statusMark;
    private readonly TextBlock _statusText;
    private readonly InfoBar _applyFailed = Cards.Notice(InfoBarSeverity.Error);
    private readonly InfoBar _stoppedRetrying = Cards.Notice(InfoBarSeverity.Error);
    private readonly AddressList _networks;
    private readonly AddressList _targets;
    private readonly Button _addCurrentNetwork = new() { Content = L("Add Current Network") };
    /// <summary>The rows a search result goes to, by the setting's catalog key.</summary>
    private readonly Dictionary<string, FrameworkElement> _rows = [];

    public ExceptionRulesPane(AppUi ui, PanePage page)
    {
        this._ui = ui;
        (_statusLine, _statusMark, _statusText) = Cards.StatusLine();
        _body.Children.Add(_rows["Current network"] = Cards.Card(L("Current network"), _currentNetwork));
        _body.Children.Add(_rows["Status"] = Cards.Card(L("Status"), _statusLine));
        _body.Children.Add(_applyFailed);
        _body.Children.Add(_stoppedRetrying);

        _networks = new AddressList(
            L("Double-click here or click + to add a network."), L("e.g. 10.20.161.0/24"), 44,
            L("Add Network"), L("Add a network"), L("Remove Selected Network"),
            rows => ui.Send(new CommandSetBypassRows(BypassList.Networks, rows)));
        _addCurrentNetwork.Click += (_, _) => ui.Send(new CommandAddCurrentNetwork());
        _body.Children.Add(Cards.SectionHeader(L("Networks where the rules apply")));
        _body.Children.Add(_rows["Networks where the rules apply"] = _networks.View);
        _rows["Add Current Network"] = _addCurrentNetwork;
        var networkButtons = new Grid { Margin = new Thickness(0, 4, 0, 0) };
        networkButtons.Children.Add(Cards.Row(_networks.Add, _networks.Remove));
        _addCurrentNetwork.HorizontalAlignment = HorizontalAlignment.Right;
        networkButtons.Children.Add(_addCurrentNetwork);
        _body.Children.Add(networkButtons);

        // A PC is used on a few networks, unlike the many exception targets.
        _targets = new AddressList(
            L("Double-click here or click + to add a host or network."), L("e.g. 10.30.161.16 or 10.30.170.0/24"), 175,
            L("Add Exception Target"), L("Add a host or network"), L("Remove Selected Exception Target"),
            rows => ui.Send(new CommandSetBypassRows(BypassList.Targets, rows)));
        _body.Children.Add(Cards.SectionHeader(L("Exception targets")));
        _body.Children.Add(_rows["Exception targets"] = _targets.View);
        StackPanel targetButtons = Cards.Row(_targets.Add, _targets.Remove);
        targetButtons.Margin = new Thickness(0, 4, 0, 0);
        _body.Children.Add(targetButtons);
        TextBlock footer = Cards.Footer(L("While this PC is on one of the networks above, the exception targets skip the VPN and go out directly through that network's router."));
        ToolTipService.SetToolTip(footer, L("Elsewhere, everything goes through the VPN. Routes are saved in Windows network settings and survive restarts."));
        _body.Children.Add(footer);
        page.BodyPanel.Children.Add(_body);
    }

    public FrameworkElement? OpenRow(string key) => _rows.GetValueOrDefault(key);

    public void Refresh()
    {
        _body.Visible(_ui.BypassRules is not null);
        if (_ui.BypassRules is not { } rules)
        {
            return;
        }
        _currentNetwork.Text = rules.CurrentNetworkText();

        // Marked addresses come first; otherwise how the rules stand on this network.
        if (rules.Networks.ProblemRows.Count > 0 || rules.Targets.ProblemRows.Count > 0)
        {
            _statusText.Text = L("Fix the marked addresses to apply.");
            _statusMark.SetStyle(NoticeStyle.Error);
        }
        else
        {
            (string? text, NoticeStyle style) = rules.Status.Line(_ui.Time);
            _statusText.Text = text;
            _statusMark.SetStyle(style);
        }
        _applyFailed.Show(rules.Notice is BypassNoticeApplyFailed failed ? failed.Error.Message() : null);
        _stoppedRetrying.Show(rules.StoppedRetryingMessage());

        _networks.Show(rules.Networks.Rows, ProblemRows(rules.Networks), RowMessages(rules, BypassList.Networks), rules.Networks.AddRow == ButtonState.Enabled);
        _targets.Show(rules.Targets.Rows, ProblemRows(rules.Targets), RowMessages(rules, BypassList.Targets), rules.Targets.AddRow == ButtonState.Enabled);
        _addCurrentNetwork.Apply(rules.AddCurrentNetwork);
    }

    public void FinishTyping()
    {
        _networks.CommitEdit();
        _targets.CommitEdit();
    }

    private static HashSet<int> ProblemRows(RuleList list) => [.. list.ProblemRows.Select(row => (int)row)];

    /// <summary>
    /// The problem the runtime explains, beside its row in <paramref name="list"/>. It reports only the
    /// first problem of the networks, or else of the targets; the other marked rows show the mark alone.
    /// </summary>
    private static Dictionary<int, (string Text, string Help)> RowMessages(BypassRulesSection rules, BypassList list)
    {
        BypassList explained = rules.Networks.ProblemRows.Count == 0 ? BypassList.Targets : BypassList.Networks;
        RuleList rows = list == BypassList.Networks ? rules.Networks : rules.Targets;
        if (list != explained || rules.Notice is not BypassNoticeRowProblem notice || rows.ProblemRows.Count == 0)
        {
            return [];
        }
        int index = (int)rows.ProblemRows[0];
        return index >= rows.Rows.Count || rows.Rows[index] != notice.Row
            ? []
            : new() { [index] = (notice.Problem.RowMessage(), notice.Problem.Message(notice.Row)) };
    }
}
