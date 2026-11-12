import AppKit
import SwiftUI

/// The Exception Rules pane. Edits are a draft until Apply, which with Discard Changes sits in the
/// toolbar (and File > Save and Revert), like Save in the Account pane.
struct ExceptionRulesPane: View {
    let ui: AppUI
    let navigation: SettingsNavigation
    @Environment(\.self) private var environment

    @State private var networkActions = AddressListActions()
    @State private var targetActions = AddressListActions()
    @State private var isEditingNetworkRow = false
    @State private var isEditingTargetRow = false

    var body: some View {
        if let rules = ui.bypassRules {
            SettingsForm(navigation: navigation) {
                Section {
                    LabeledContent("Current network") {
                        Text(verbatim: rules.currentNetworkText)
                            .textSelection(.enabled)
                    }
                    .searchAnchor("Current network", navigation)
                }
                networksSection(rules)
                targetsSection(rules)
                statusSection(rules, time: TimeStyle(now: ui.clock.now, environment: environment))
            }
            .toolbar {
                ToolbarItemGroup(placement: .primaryAction) {
                    StateButton("Discard Changes", state: rules.whileEditing(rules.discard, navigation.isEditingRuleRow)) {
                        ui.discardBypassChanges()
                    }
                    StateButton("Apply", state: rules.whileEditing(rules.apply, navigation.isEditingRuleRow)) {
                        ui.applyBypassRules()
                    }
                    .buttonStyle(.borderedProminent)
                    .help("Apply exception rules (⌘S)")
                }
            }
        }
    }

    private func networksSection(_ rules: AppInternal.BypassRulesSection) -> some View {
        Section("Networks where the rules apply") {
            StateButton("Add Current Network", state: rules.addCurrentNetwork) { ui.send(.addCurrentNetwork) }
                .searchAnchor("Add Current Network", navigation)
            AddressListEditor(
                rows: rules.networks.rows,
                problemRows: Set(rules.networks.problemRows),
                problemMessages: rules.rowMessages(in: .networks),
                canAdd: rules.networks.addRow == .enabled,
                emptyText: String(localized: "Double-click here or click + to add a network."),
                placeholder: String(localized: "e.g. 10.20.161.0/24"),
                actions: networkActions,
                onChange: { ui.send(.setBypassRows(list: .networks, rows: $0)) },
                onEditingChanged: { isEditingNetworkRow = $0; editingChanged() }
            )
            .spanningFormRow()
            .frame(height: 105)
            .searchAnchor("Networks where the rules apply", navigation)
            AddressListButtons(
                actions: networkActions, canAdd: rules.networks.addRow == .enabled,
                addTitle: "Add Network", removeTitle: "Remove Selected Network",
                addHelp: String(localized: "Add a network")
            )
        }
    }

    private func targetsSection(_ rules: AppInternal.BypassRulesSection) -> some View {
        Section {
            AddressListEditor(
                rows: rules.targets.rows,
                problemRows: Set(rules.targets.problemRows),
                problemMessages: rules.rowMessages(in: .targets),
                canAdd: rules.targets.addRow == .enabled,
                emptyText: String(localized: "Double-click here or click + to add a host or network."),
                placeholder: String(localized: "e.g. 10.30.161.16 or 10.30.170.0/24"),
                actions: targetActions,
                onChange: { ui.send(.setBypassRows(list: .targets, rows: $0)) },
                onEditingChanged: { isEditingTargetRow = $0; editingChanged() }
            )
            .spanningFormRow()
            .frame(height: 175)
            .searchAnchor("Exception targets", navigation)
            AddressListButtons(
                actions: targetActions, canAdd: rules.targets.addRow == .enabled,
                addTitle: "Add Exception Target", removeTitle: "Remove Selected Exception Target",
                addHelp: String(localized: "Add a host or network")
            )
        } header: {
            Text("Exception targets")
        } footer: {
            Text("While this Mac is on one of the networks above, the exception targets skip the VPN and go out directly through that network's router.")
                .footnoteStyle()
                .help("Elsewhere, everything goes through the VPN. Routes are saved in macOS network settings and survive restarts.")
        }
    }

    /// Marked addresses and unapplied changes come first; otherwise how the rules stand on this
    /// network, as the runtime reports it.
    private func statusSection(_ rules: AppInternal.BypassRulesSection, time: TimeStyle) -> some View {
        Section {
            LabeledContent("Status") {
                if !rules.networks.problemRows.isEmpty || !rules.targets.problemRows.isEmpty {
                    Label("Fix the marked addresses to apply.", systemImage: "exclamationmark.circle.fill")
                        .foregroundStyle(.red)
                } else if rules.hasChanges {
                    Label("There are changes that haven't been applied.", systemImage: "pencil.circle")
                        .foregroundStyle(.secondary)
                } else {
                    let status = rules.status.line(time: time)
                    NoticeRow(text: status.text, style: status.style)
                }
            }
            .searchAnchor("Status", navigation)
            if case let .applyFailed(error)? = rules.notice {
                NoticeRow(text: error.message, style: .error)
            }
            if let message = rules.stoppedRetryingMessage {
                NoticeRow(text: message, style: .error)
            }
        }
    }

    private func editingChanged() {
        navigation.isEditingRuleRow = isEditingNetworkRow || isEditingTargetRow
    }
}

extension AppInternal.BypassRulesSection {
    /// The problem the runtime explains, beside its row in `list`. It reports only the first problem
    /// of the networks, or else of the targets; the other marked rows show the mark alone.
    func rowMessages(in list: AppInternal.BypassList) -> [Int: AddressListEditor.RowMessage] {
        let explained: AppInternal.BypassList = networks.problemRows.isEmpty ? .targets : .networks
        let rules = list == .networks ? networks : targets
        guard list == explained, case let .rowProblem(row, problem)? = notice,
              let index = rules.problemRows.first, index < rules.rows.count, rules.rows[index] == row
        else { return [:] }
        return [index: AddressListEditor.RowMessage(text: problem.rowMessage, help: problem.message(for: row))]
    }
}

private extension View {
    /// The list spans its form row edge to edge, as the settings mockup's list does.
    func spanningFormRow() -> some View {
        padding(.horizontal, -AddressListEditorView.textInset)
    }
}
