import AppKit
import SwiftUI

struct SoftwareUpdatePage: View {
    let ui: AppUI
    let navigation: SettingsNavigation

    var body: some View {
        SettingsForm(navigation: navigation) {
            if let update = ui.autoUpdate {
                Section {
                    switch update.progress {
                    case let .readyToRestart(version):
                        restartRows(update, version: version, installing: false)
                    case let .installing(version):
                        restartRows(update, version: version, installing: true)
                    case .idle, .downloading, .preparing, .failed:
                        statusRows(update)
                    }
                }
                let available = if case .available = update.status { true } else { false }
                if available || update.progress != .idle {
                    Section {
                        LabeledContent("Installed") {
                            Text(verbatim: "FortiVPNManager \(update.currentVersion)")
                        }
                    }
                }
            }
        }
        .task(id: navigation.isWindowOpen) {
            if navigation.isWindowOpen { ui.send(.updatePageOpened) }
        }
    }

    @ViewBuilder
    private func statusRows(_ update: AppInternal.AutoUpdateSection) -> some View {
        switch (update.status, update.progress) {
        case (.checking, _):
            statusRow(update, title: Text("Checking for updates…")) {
                ProgressView()
            } trailing: {
                StateButton("Check for Updates", state: update.check) { ui.send(.checkForUpdates) }
            }
        case (.upToDate, _):
            statusRow(update, title: Text("FortiVPNManager is up to date."), detail: "FortiVPNManager \(update.currentVersion)") {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 22))
                    .symbolRenderingMode(.palette)
                    .foregroundStyle(.green, .white)
                    .frame(width: 36, height: 36)
                    .background(.green, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            } trailing: {
                StateButton("Check for Updates", state: update.check) { ui.send(.checkForUpdates) }
            }
        case let (.checkFailed(reason), _):
            statusRow(update, title: Text(verbatim: reason.message), detail: "FortiVPNManager \(update.currentVersion)") {
                appIcon
            } trailing: {
                StateButton("Check for Updates", state: update.check) { ui.send(.checkForUpdates) }
            }
        case let (.available(version, _, _, _), .downloading(received, total, percent, remaining)):
            statusRow(
                update, title: Text("Downloading “FortiVPNManager \(version)”…"), percent: percent,
                detail: AppInternal.AutoUpdateSection.downloadLine(received: received, total: total, remaining: remaining)
            ) {
                appIcon
            } trailing: {
                EmptyView()
            }
        case let (.available, .preparing(percent)):
            statusRow(update, title: Text("Preparing update…"), percent: percent) {
                appIcon
            } trailing: {
                EmptyView()
            }
        case let (.available(version, _, installerBytes, _), progress):
            let failed = if case .failed = progress { true } else { false }
            statusRow(
                update, title: Text(verbatim: "FortiVPNManager \(version)"),
                detail: "\(version) — \(AppInternal.AutoUpdateSection.size(installerBytes))"
            ) {
                appIcon
            } trailing: {
                StateButton(failed ? "Try Again" : "Update Now", state: update.updateNow) { ui.send(.updateNow) }
            }
        }
        if case let .failed(reason) = update.progress {
            NoticeRow(text: reason.message, style: .error)
        }
        if let failed = update.failedInstallMessage {
            NoticeRow(text: failed, style: .error)
        }
        switch (update.status, update.progress) {
        case let (.available(_, releasePageUrl, _, _), .idle), let (.available(_, releasePageUrl, _, _), .failed):
            let titles = update.status.releaseNoteTitles(in: Bundle.main.preferredLocalizations[0])
            if !titles.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("What's New")
                        .font(.headline)
                    ForEach(Array(titles.enumerated()), id: \.offset) { _, title in
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Text(verbatim: "•")
                            Text(verbatim: title)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
            }
            Text(installNote(releasePage: URL(string: releasePageUrl)!))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        default:
            EmptyView()
        }
    }

    private func statusRow<Icon: View, Trailing: View>(
        _ update: AppInternal.AutoUpdateSection, title: Text, percent: Int? = nil, detail: String? = nil,
        @ViewBuilder icon: () -> Icon, @ViewBuilder trailing: () -> Trailing
    ) -> some View {
        HStack(spacing: 12) {
            icon()
                .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: 5) {
                title
                    .fixedSize(horizontal: false, vertical: true)
                if let percent {
                    HStack(spacing: 8) {
                        ProgressView(value: Double(percent), total: 100)
                        if update.cancel != .hidden {
                            Button {
                                ui.send(.cancelUpdateDownload)
                            } label: {
                                Image(systemName: "xmark.circle.fill")
                            }
                            .buttonStyle(.plain)
                            .foregroundStyle(.secondary)
                            .disabled(update.cancel == .disabled)
                            .accessibilityLabel("Cancel Download")
                            .help("Cancel Download")
                        }
                    }
                }
                if let detail {
                    Text(verbatim: detail)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trailing()
                .fixedSize()
        }
    }

    @ViewBuilder
    private func restartRows(_ update: AppInternal.AutoUpdateSection, version: String, installing: Bool) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text(installing ? "Installing…" : "Update Ready")
                    .font(.headline)
                if installing {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                }
                Spacer()
                StateButton("Restart App", state: update.restart) { ui.send(.restartToUpdate) }
                    .fixedSize()
            }
            Text(verbatim: "• FortiVPNManager \(version)")
        }
        Text(installing
            ? "The app quits to install the update and opens again when it's done."
            : "The update installs when you restart the app. The VPN disconnects briefly during the install.")
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var appIcon: some View {
        Image(nsImage: NSApp.applicationIconImage)
            .resizable()
    }

    private func installNote(releasePage: URL) -> AttributedString {
        var link = AttributedString(String(localized: "View on GitHub…"))
        link.link = releasePage
        return AttributedString(String(localized: "The VPN disconnects briefly during the install.") + " ") + link
    }
}
