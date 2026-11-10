import AppKit
import SwiftUI

/// The Exception Rules pane. Edits are a draft until Apply, which with Discard Changes sits in the
/// bar at the bottom, like Save in the Account pane.
struct ExceptionRulesPane: View {
    let ui: AppUI
    @Environment(\.self) private var environment

    /// A row of a list is being typed: its text reaches the runtime when the edit ends, which Apply
    /// and Discard Changes bring about first, so they take clicks meanwhile.
    @State private var isEditingNetworkRow = false
    @State private var isEditingTargetRow = false

    var body: some View {
        VStack(spacing: 0) {
            if let rules = ui.bypassRules {
                Form {
                    rulesSection(rules, time: TimeStyle(now: ui.clock.now, environment: environment))
                }
                .formStyle(.grouped)
                Divider()
                bottomBar(rules)
                    .padding(12)
            }
        }
    }

    private func rulesSection(_ rules: AppInternal.BypassRulesSection, time: TimeStyle) -> some View {
        Section {
            LabeledContent("Current network", value: rules.currentNetworkText)
            LabeledContent("Networks where the rules apply") {
                StateButton("Add Current Network", state: rules.addCurrentNetwork) { ui.send(.addCurrentNetwork) }
            }
            AddressListEditor(
                rows: rules.networks.rows,
                problemRows: Set(rules.networks.problemRows),
                canAdd: rules.networks.addRow == .enabled,
                emptyText: String(localized: "Double-click here or click + to add a network such as 10.20.161.0/24."),
                placeholder: String(localized: "e.g. 10.20.161.0/24"),
                addHelp: String(localized: "Add a network"),
                onChange: { ui.send(.setBypassRows(list: .networks, rows: $0)) },
                onEditingChanged: { isEditingNetworkRow = $0 }
            )
            .frame(height: 96)
            Text("Exception targets")
            AddressListEditor(
                rows: rules.targets.rows,
                problemRows: Set(rules.targets.problemRows),
                canAdd: rules.targets.addRow == .enabled,
                emptyText: String(localized: "Double-click here or click + to add a host or network."),
                placeholder: String(localized: "e.g. 10.30.161.16 or 10.30.170.0/24"),
                addHelp: String(localized: "Add a host or network"),
                onChange: { ui.send(.setBypassRows(list: .targets, rows: $0)) },
                onEditingChanged: { isEditingTargetRow = $0 }
            )
            .frame(height: 180)
            if let notice = rules.notice {
                NoticeRow(text: notice.message, style: .error)
            }
            LabeledContent("Status") {
                let status = rules.status.line(time: time)
                NoticeRow(text: status.text, style: status.style)
            }
            if let message = rules.stoppedRetryingMessage {
                NoticeRow(text: message, style: .error)
            }
        } footer: {
            Text("While this Mac is on one of the networks above, the exception targets skip the VPN and go out directly through that network's router.")
                .footnoteStyle()
                .help("On any other network everything goes through the VPN. The routes are saved in the macOS network settings, so they stay after a restart. On a listed network, routes added elsewhere through the same router appear here; routes through other gateways are left alone.")
        }
    }

    /// No default button: Return in a list edits its selected row.
    private func bottomBar(_ rules: AppInternal.BypassRulesSection) -> some View {
        let isEditing = isEditingNetworkRow || isEditingTargetRow
        return HStack(spacing: 10) {
            if rules.hasChanges {
                Text("You have changes that aren't applied yet.")
                    .footnoteStyle()
            }
            Spacer()
            StateButton("Discard Changes", state: whileEditing(rules.discard, isEditing)) {
                endTextEditing()
                ui.send(.discardBypassChanges)
            }
            StateButton("Apply", state: whileEditing(rules.apply, isEditing)) {
                endTextEditing()
                ui.send(.applyBypassRules)
            }
        }
    }

    private func whileEditing(_ state: AppInternal.ButtonState, _ isEditing: Bool) -> AppInternal.ButtonState {
        isEditing && state == .disabled ? .enabled : state
    }

    /// Finishes a row being typed so its text is sent first (buttons do not take focus).
    private func endTextEditing() {
        NSApp.keyWindow?.makeFirstResponder(nil)
    }
}
