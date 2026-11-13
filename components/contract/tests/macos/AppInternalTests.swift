import Foundation
import Testing

/// Checks the Swift types in components/contract/macos/app against the JSON in
/// ../fixtures/app-internal, which the Rust tests pin: Swift decodes what Rust sends into the values
/// below and encodes what it sends itself into the same JSON. Coding goes through `MessageCoding`,
/// as on the stream.
struct AppInternalTests {
    private func fixture(_ name: String) throws -> Data {
        let tests = URL(filePath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        return try Data(contentsOf: tests.appending(path: "fixtures/app-internal/\(name)"))
    }

    private func decoded<T: Decodable>(_ name: String) throws -> T {
        try MessageCoding.decoder().decode(T.self, from: fixture(name))
    }

    private func encodesLikeFixture(_ messages: [AppInternal.ToRuntime], _ name: String) throws -> Bool {
        let encoded = try JSONSerialization.jsonObject(with: MessageCoding.encoder().encode(messages)) as? NSArray
        let expected = try JSONSerialization.jsonObject(with: fixture(name)) as? NSArray
        return encoded != nil && encoded == expected
    }

    // MARK: Rust-sent

    @Test func messagesFromTheRuntime() throws {
        let messages: [AppInternal.FromRuntime] = try decoded("from_runtime.json")
        #expect(messages == AppInternal.Expected.fromRuntime)
    }

    @Test func connectionSections() throws {
        let sections: [AppInternal.ConnectionSection] = try decoded("connection_sections.json")
        #expect(sections == AppInternal.Expected.connectionSections)
    }

    @Test func connectionKinds() throws {
        let progress: [AppInternal.ConnectProgress] = try decoded("connect_progress.json")
        #expect(progress == [
            .checkingReadiness, .connectingToGmail, .contactingServer, .loggingIn, .waitingForOtp, .verifyingOtp,
            .openingTunnel,
        ])
        let afterDisconnect: [AppInternal.AfterDisconnect] = try decoded("after_disconnect.json")
        #expect(afterDisconnect == [.idle, .reconnect, .sleep, .quit])
        let reasons: [AppInternal.FailureReason] = try decoded("failure_reasons.json")
        #expect(reasons == AppInternal.Expected.failureReasons)
        let waits: [AppInternal.WaitReason] = try decoded("wait_reasons.json")
        #expect(waits == AppInternal.Expected.waitReasons)
        let items: [AppInternal.SetupItem] = try decoded("setup_items.json")
        #expect(items == [
            .configFile, .accountDetails, .vpnPassword, .gmailCredential(method: .imap), .helperApproval, .helperVersion,
        ])
    }

    @Test func scheduleSections() throws {
        let sections: [AppInternal.ScheduleSection] = try decoded("schedule_sections.json")
        #expect(sections == AppInternal.Expected.scheduleSections)
    }

    @Test func readinessSections() throws {
        let sections: [AppInternal.ReadinessSection] = try decoded("readiness_sections.json")
        #expect(sections == AppInternal.Expected.readinessSections)
        let problems: [AppInternal.ClientProblem] = try decoded("client_problems.json")
        let cellar = "/opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn"
        #expect(problems == [
            .init(kind: .notFound, path: cellar),
            .init(kind: .outsideAllowedLocation, path: cellar),
            .init(kind: .notRegularFile, path: cellar),
            .init(kind: .writableByOthers(mode: 0o775), path: cellar),
            .init(kind: .notExecutable, path: cellar),
        ])
        let failures: [AppInternal.HelperChannelFailure] = try decoded("helper_channel_failures.json")
        #expect(failures == AppInternal.Expected.helperChannelFailures)
    }

    @Test func gmailSections() throws {
        let sections: [AppInternal.GmailSection] = try decoded("gmail_sections.json")
        #expect(sections == AppInternal.Expected.gmailSections)
        let blockers: [AppInternal.GoogleConnectBlocker] = try decoded("google_connect_blockers.json")
        #expect(blockers == [.noOAuthClient, .enterAddress, .saveAddressFirst])
        let errors: [AppInternal.GoogleError] = try decoded("google_errors.json")
        #expect(errors == AppInternal.Expected.googleErrors)
        let tests: [AppInternal.GmailTest] = try decoded("gmail_tests.json")
        #expect(tests == AppInternal.Expected.gmailTests)
    }

    @Test func bypassRulesSections() throws {
        let sections: [AppInternal.BypassRulesSection] = try decoded("bypass_rules_sections.json")
        #expect(sections == AppInternal.Expected.bypassRulesSections)
    }

    @Test func configSections() throws {
        let sections: [AppInternal.ConfigSection] = try decoded("config_sections.json")
        #expect(sections == AppInternal.Expected.configSections)
    }

    @Test func settingsFileProblems() throws {
        let errors: [AppInternal.ConfigFileError] = try decoded("config_file_errors.json")
        #expect(errors == AppInternal.Expected.configFileErrors)
        let problems: [AppInternal.ConfigProblem] = try decoded("config_problems.json")
        #expect(problems == AppInternal.Expected.configProblems)
        let warnings: [AppInternal.ConfigWarning] = try decoded("config_warnings.json")
        #expect(warnings == AppInternal.Expected.configWarnings)
        let addresses: [AppInternal.AddressProblem] = try decoded("address_problems.json")
        #expect(addresses == [
            .malformed, .prefixOutOfRange(length: 4, minimum: 8, maximum: 32), .nonContiguousMask, .unroutable,
            .hostBitsSet(network: "10.30.170.0/24"),
        ])
        let files: [AppInternal.FileProblem] = try decoded("file_problems.json")
        #expect(files == [
            .notFound, .notRegularFile, .tooLarge(limitBytes: 65536),
            .io(operation: "open", path: "/Users/alice/Downloads/client_secret.json", errno: 1),
        ])
    }

    @Test func portRequests() throws {
        let messages: [AppInternal.FromRuntime] = try decoded("port_requests.json")
        #expect(messages == AppInternal.Expected.portRequests)
    }

    @Test func portActions() throws {
        let messages: [AppInternal.FromRuntime] = try decoded("port_actions.json")
        #expect(messages == AppInternal.Expected.portActions)
    }

    // MARK: Swift-sent

    @Test func launched() throws {
        #expect(try encodesLikeFixture(AppInternal.Expected.launched, "launched.json"))
    }

    @Test func commands() throws {
        #expect(try encodesLikeFixture(AppInternal.Expected.commands, "commands.json"))
    }

    @Test func portResponses() throws {
        #expect(try encodesLikeFixture(AppInternal.Expected.portResponses, "port_responses.json"))
    }

    @Test func systemEvents() throws {
        #expect(try encodesLikeFixture(AppInternal.Expected.systemEvents, "system_events.json"))
    }

    // MARK: Keychain

    @Test func secretsKeepTheirKeychainItems() {
        let items: [AppInternal.SecretItem] = [
            .vpnPassword(username: "alice", host: "vpn.example.com"),
            .gmailAppPassword(address: "Alice@example.com"),
            .googleRefreshToken(address: "Alice@Example.com"),
            .googleClientSecret(clientId: "987-xyz.apps.googleusercontent.com"),
        ]
        #expect(items.map(\.keychainService) == [
            "com.qoor.FortiVPNManager.vpn",
            "com.qoor.FortiVPNManager.gmail",
            "com.qoor.FortiVPNManager.google",
            "com.qoor.FortiVPNManager.google",
        ])
        #expect(items.map(\.keychainAccount) == [
            "alice@vpn.example.com",
            "Alice@example.com",
            "refresh-token:alice@example.com",
            "client-secret:987-xyz.apps.googleusercontent.com",
        ])
    }
}

/// The values the fixtures hold, written inside the namespace so the nested type names resolve.
private extension AppInternal {
    enum Expected {
        static let attempt = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F"
        static let t0 = 1_762_412_345_250
        static let configPath = "/Users/alice/.config/FortiVPNManager/config.yaml"

        /// Milliseconds since 1970, as the decoder reads them.
        static func at(_ millis: Int) -> Date {
            Date(timeIntervalSince1970: Double(millis) / 1000)
        }

        static let storeFailure = SecretStoreFailure(description: "User interaction is not allowed.")

        static let unreadableFile = ConfigFileError.unreadableFile(
            path: configPath,
            problem: ConfigProblem(kind: .duplicateKey(key: "vpn"), location: LineColumn(line: 7, column: 1))
        )

        static func connectionSection(_ status: ConnectionStatus) -> ConnectionSection {
            ConnectionSection(
                icon: .disconnected, status: status, waitReason: nil, connect: .enabled, disconnect: .hidden,
                reconnectNow: .disabled, accountAppliesNextConnection: false
            )
        }

        static func scheduleSection(enabled: Bool, outlook: ScheduleOutlook?, hoursError: ScheduleHoursError?) -> ScheduleSection {
            ScheduleSection(
                enabled: enabled, days: Weekday.allCases, start: TimeOfDay(hour: 9, minute: 0),
                end: TimeOfDay(hour: 18, minute: 30), awayMinutes: MinutesSetting(value: 60, minimum: 1, maximum: 1440),
                outlook: outlook, hoursError: hoursError
            )
        }

        static func readinessSection(_ helper: HelperState) -> ReadinessSection {
            ReadinessSection(helper: helper, registerHelper: .hidden, registerError: nil, client: nil)
        }

        static func gmailSection(_ test: GmailTest, account: GoogleAccountStatus = .notConnected) -> GmailSection {
            GmailSection(
                method: .gmailApi, methodError: nil, googleAccount: account, connectGoogle: .enabled,
                disconnectGoogle: .hidden, cancelSignIn: .hidden, connectBlocker: nil, googleError: nil,
                oauthClient: .builtIn(clientId: "123-abc.apps.googleusercontent.com"),
                importedClientSecretMissing: false, useBuiltInClient: .hidden, importClient: .enabled, test: test,
                testButton: .enabled
            )
        }

        static func bypassSection(
            _ status: BypassStatus, notice: BypassNotice?, currentNetwork: CurrentNetwork? = nil
        ) -> BypassRulesSection {
            BypassRulesSection(
                currentNetwork: currentNetwork, networks: RuleList(rows: [], problemRows: [], addRow: .enabled),
                targets: RuleList(rows: [], problemRows: [], addRow: .enabled), notice: notice, status: status,
                hasChanges: false,
                addCurrentNetwork: .disabled, discard: .disabled, apply: .disabled
            )
        }

        static func accountForm(
            _ revision: UInt64, feedback: AccountFeedback? = nil, importReport: ClientSettingsImport? = nil
        ) -> AccountFormState {
            AccountFormState(
                revision: revision, host: "vpn.example.com", port: "443", username: "alice", realm: "",
                gmailAddress: "alice@example.com", vpnPasswordEntered: false, appPasswordEntered: false,
                vpnPasswordStored: true, appPasswordStored: false, savedAccountProblem: nil, feedback: feedback,
                save: .disabled, revert: .disabled, importReport: importReport, defaultImportFile: nil
            )
        }

        static func configSection(_ account: AccountFormState, optionsError: ConfigFileError? = nil) -> ConfigSection {
            ConfigSection(
                file: ConfigFileState(path: configPath, exists: true, loadError: nil, warnings: []),
                account: account,
                options: ConnectionOptions(
                    autoConnectOnLaunch: true, sessionMaxMinutes: MinutesSetting(value: 180, minimum: 10, maximum: 1440),
                    proactiveReconnect: true, marginMinutes: MinutesSetting(value: 5, minimum: 1, maximum: 60),
                    reconnectAfterSleep: false, minSleepMinutes: MinutesSetting(value: 30, minimum: 1, maximum: 1440)
                ),
                optionsError: optionsError,
                openAtLogin: OpenAtLogin(available: true, enabled: true, requiresApproval: false, error: nil),
                logFile: "/Users/alice/Library/Logs/FortiVPNManager/openfortivpn.log"
            )
        }

        static func importReport(_ password: ImportedPassword?) -> ImportReport {
            ImportReport(
                source: "/Users/alice/.config/openfortivpn/config", host: "vpn.example.com", port: 10443,
                username: "alice", realm: "staff", trustedCerts: 1, password: password, unknownKeys: ["insecure-cert"],
                ignoredKeys: ["set-dns"], unreadableLines: [4, 9]
            )
        }

        // MARK: Rust-sent

        static let fromRuntime: [FromRuntime] = [
            .section(.connection(connectionSection(.notConnected))),
            .section(.schedule(scheduleSection(enabled: false, outlook: nil, hoursError: nil))),
            .section(.readiness(ReadinessSection(helper: .checking, registerHelper: .hidden, registerError: nil, client: nil))),
            .section(.gmail(gmailSection(.idle))),
            .section(.bypassRules(bypassSection(.checking, notice: nil))),
            .section(.config(configSection(accountForm(0)))),
            .logLines(lines: [
                LogLine(time: at(t0), source: .stdout, text: "INFO:   Connected to gateway."),
                LogLine(time: at(t0 + 750), source: .stderr, text: "Using interface ppp0"),
                LogLine(time: at(t0 + 1_000), source: .note, text: "‹schedule: scheduled hours started, connecting›"),
            ]),
            .portRequest(id: 7, request: .screenState),
            .portAction(.approveQuit),
            .openSettings,
        ]

        static let connectionSections: [ConnectionSection] = [
            ConnectionSection(
                icon: .connected,
                status: .connected(
                    since: at(t0), tunnelIp: "10.211.0.7", proactiveReconnect: .at(at: at(t0 + 10_500_000))
                ),
                waitReason: nil, connect: .hidden, disconnect: .enabled, reconnectNow: .enabled,
                accountAppliesNextConnection: true
            ),
            connectionSection(.connected(since: at(t0), tunnelIp: nil, proactiveReconnect: .pausedBySchedule)),
            connectionSection(.connected(since: at(t0), tunnelIp: nil, proactiveReconnect: .off)),
            ConnectionSection(
                icon: .inProgress, status: .waitingToReconnect(until: at(t0 + 90_000)),
                waitReason: .authCooldown(delaySeconds: 90), connect: .hidden, disconnect: .enabled,
                reconnectNow: .enabled, accountAppliesNextConnection: false
            ),
            ConnectionSection(
                icon: .error, status: .setupRequired(items: [.vpnPassword, .gmailCredential(method: .gmailApi)]),
                waitReason: nil, connect: .enabled, disconnect: .enabled, reconnectNow: .disabled,
                accountAppliesNextConnection: false
            ),
            ConnectionSection(
                icon: .error, status: .failed(reason: .vpnLoginRejected), waitReason: nil, connect: .enabled,
                disconnect: .hidden, reconnectNow: .disabled, accountAppliesNextConnection: false
            ),
            connectionSection(.connecting(reconnecting: true, progress: .waitingForOtp)),
            ConnectionSection(
                icon: .disconnected, status: .reconnectOnHold(until: at(t0 + 1_800_000)), waitReason: .userAway,
                connect: .enabled, disconnect: .hidden, reconnectNow: .disabled, accountAppliesNextConnection: false
            ),
            connectionSection(.waitingForNetwork),
            connectionSection(.disconnecting(then: .reconnect)),
        ]

        static let failureReasons: [FailureReason] = [
            .vpnLoginRejected,
            .otpNotReceived(inSpam: false, waitedSeconds: 50),
            .otpRejected,
            .tunnelSetupFailed,
            .tunnelModeRejected,
            .droppedEarly(withinSeconds: 120),
            .certificateChangedAfterLogin,
            .gmailRejected(method: .imap),
            .tooManyReconnects(windowSeconds: 3600),
            .network,
            .helper(code: .busy),
            .helper(code: .invalidRequest),
            .helper(code: .unknownAttempt),
            .helper(code: .wrongPhase),
            .helper(code: .clientUnavailable),
            .helper(code: .notAuthorized),
            .helper(code: .internalError),
        ]

        static let waitReasons: [WaitReason] = [
            .network,
            .serverEndedSession,
            .forcedTermination,
            .authFailure(reason: .otpNotReceived(inSpam: true, waitedSeconds: 50)),
            .authCooldown(delaySeconds: 90),
            .sleep,
            .userAway,
        ]

        static let scheduleSections: [ScheduleSection] = [
            scheduleSection(enabled: true, outlook: .connects(at: at(t0 + 3_600_000)), hoursError: nil),
            scheduleSection(enabled: true, outlook: .noDays, hoursError: .endNotAfterStart),
            scheduleSection(
                enabled: true, outlook: .withinHours(end: at(t0 + 7_200_000)), hoursError: .notSaved(error: unreadableFile)
            ),
            scheduleSection(enabled: true, outlook: .outsideHoursPresent, hoursError: nil),
            scheduleSection(enabled: true, outlook: .holdingWhileAway(until: at(t0 + 1_800_000)), hoursError: nil),
            scheduleSection(enabled: true, outlook: .suspended, hoursError: nil),
        ]

        static let readinessSections: [ReadinessSection] = [
            ReadinessSection(
                helper: .running(build: "20261109120000"), registerHelper: .hidden, registerError: nil,
                client: VpnClientInfo(
                    path: "/opt/homebrew/Cellar/openfortivpn/1.25.0/bin/openfortivpn", version: "1.25.0",
                    untestedVersion: UntestedVersion(testedSeries: "1.24"), problem: nil
                )
            ),
            ReadinessSection(
                helper: .notRegistered, registerHelper: .enabled, registerError: .notInstalledCopy, client: nil
            ),
            ReadinessSection(
                helper: .running(build: "20261109120000"), registerHelper: .hidden,
                registerError: .failed(description: "Operation not permitted"),
                client: VpnClientInfo(
                    path: nil, version: nil, untestedVersion: nil,
                    problem: ClientProblem(kind: .notFound, path: "/opt/homebrew/bin/openfortivpn")
                )
            ),
            readinessSection(.requiresApproval),
            readinessSection(.versionMismatch(helperBuild: "20261108090000", appBuild: "20261109120000")),
            readinessSection(.unreachable(failure: .noReply)),
        ]

        static let helperChannelFailures: [HelperChannelFailure] = [
            .appNotSigned,
            .connection(description: "Couldn’t communicate with a helper application."),
            .connectionLost,
            .noReply,
            .unreadableReply(description: "The data couldn’t be read because it isn’t in the correct format."),
        ]

        static let gmailSections: [GmailSection] = [
            GmailSection(
                method: .gmailApi, methodError: nil, googleAccount: .signingIn, connectGoogle: .hidden,
                disconnectGoogle: .hidden, cancelSignIn: .enabled, connectBlocker: nil, googleError: nil,
                oauthClient: .imported(clientId: "987-xyz.apps.googleusercontent.com"),
                importedClientSecretMissing: false, useBuiltInClient: .disabled, importClient: .disabled, test: .idle,
                testButton: .disabled
            ),
            GmailSection(
                method: .imap, methodError: unreadableFile, googleAccount: .notConnected, connectGoogle: .disabled,
                disconnectGoogle: .hidden, cancelSignIn: .hidden, connectBlocker: .saveAddressFirst,
                googleError: .wrongAccount(expected: "alice@example.com", actual: "bob@example.com"),
                oauthClient: .notConfigured, importedClientSecretMissing: true, useBuiltInClient: .hidden,
                importClient: .enabled, test: .running, testButton: .disabled
            ),
            gmailSection(.apiSucceeded(address: "alice@example.com"), account: .disconnecting),
            gmailSection(
                .imapSucceeded(archive: "[Gmail]/All Mail", trash: "[Gmail]/Trash", spam: "[Gmail]/Spam"),
                account: .connected(address: "alice@example.com")
            ),
        ]

        static let googleErrors: [GoogleError] = [
            .clientFileUnreadable(problem: .tooLarge(limitBytes: 65536)),
            .clientFileInvalid(problem: .notJson),
            .clientFileInvalid(problem: .webClient),
            .clientFileInvalid(problem: .noDesktopClient),
            .clientFileInvalid(problem: .missingField(name: "client_secret")),
            .clientFileInvalid(problem: .invalidField(name: "client_id")),
            .listenerFailed(detail: "Address already in use (os error 48)"),
            .browserUnavailable,
            .timedOut(afterSeconds: 300),
            .cancelledInBrowser,
            .denied(code: "invalid_scope"),
            .tokenRejected(detail: "Bad Request"),
            .clientRejected,
            .noRefreshToken,
            .scopeNotGranted,
            .wrongAccount(expected: "alice@example.com", actual: "bob@example.com"),
            .gmailApiRefused(reason: "accessNotConfigured"),
            .network(detail: "connection refused"),
            .secretStore(failure: storeFailure),
            .config(error: .keptChanging(path: configPath)),
        ]

        static let gmailTests: [GmailTest] = [
            .idle,
            .running,
            .imapSucceeded(archive: "[Gmail]/All Mail", trash: "[Gmail]/Trash", spam: nil),
            .apiSucceeded(address: "alice@example.com"),
            .failed(failure: .enterAddress),
            .failed(failure: .enterAppPassword),
            .failed(failure: .appPasswordUnreadable(failure: storeFailure)),
            .failed(failure: .imapLoginRejected(serverText: "[AUTHENTICATIONFAILED] Invalid credentials (Failure)")),
            .failed(failure: .imapLoginRejected(serverText: nil)),
            .failed(failure: .mailboxesMissing(allMail: true, trash: false)),
            .failed(failure: .imapUnreachable(failure: .network(detail: "dns error: failed to lookup address information"))),
            .failed(failure: .imapUnreachable(failure: .timedOut(step: .connecting))),
            .failed(failure: .imapUnreachable(failure: .timedOut(step: .command(name: "UID SEARCH")))),
            .failed(failure: .imapUnreachable(failure: .serverClosed(text: "System error"))),
            .failed(failure: .imapUnreachable(failure: .serverClosed(text: nil))),
            .failed(failure: .imapUnreachable(failure: .protocol(detail: "unexpected server greeting"))),
            .failed(failure: .imapUnreachable(failure: .rejected(
                command: "SELECT", status: "NO", text: "[NONEXISTENT] Unknown Mailbox"
            ))),
            .failed(failure: .noOAuthClient),
            .failed(failure: .notSignedIn),
            .failed(failure: .accessRevoked),
            .failed(failure: .clientRejected),
            .failed(failure: .secretStore(failure: storeFailure)),
            .failed(failure: .otherAccount(authorized: "bob@example.com", expected: "alice@example.com")),
            .failed(failure: .accessTokenRejected),
            .failed(failure: .gmailApiRefused(reason: "accessNotConfigured")),
            .failed(failure: .rateLimited),
            .failed(failure: .googleUnreachable(detail: "connection reset by peer")),
            .failed(failure: .googleReturnedError(detail: "HTTP 500")),
        ]

        static let bypassRulesSections: [BypassRulesSection] = [
            BypassRulesSection(
                currentNetwork: CurrentNetwork(
                    serviceName: "Ethernet", address: "10.20.161.20", prefixLength: 24, within: "10.20.161.0/24"
                ),
                networks: RuleList(rows: ["10.20.161.0/24"], problemRows: [], addRow: .enabled),
                targets: RuleList(
                    rows: ["10.30.161.16", "10.30.161", "10.30.161.16/32"], problemRows: [1, 2], addRow: .disabled
                ),
                notice: .rowProblem(row: "10.30.161", problem: .invalid(problem: .malformed)),
                status: .applied(
                    serviceName: "Ethernet", router: "10.20.161.1", onLinkTargets: 1, notImported: 2,
                    keptThroughOtherGateways: 3
                ),
                hasChanges: true, addCurrentNetwork: .disabled, discard: .enabled, apply: .disabled
            ),
            bypassSection(
                .outsideNetworks(keptThroughOtherGateways: 0),
                notice: .rowProblem(row: "10.30.161.16/32", problem: .duplicate(entry: "10.30.161.16")),
                currentNetwork: CurrentNetwork(serviceName: "Wi-Fi", address: "192.168.0.12", prefixLength: nil, within: nil)
            ),
            bypassSection(
                .noNetworksListed(keptThroughOtherGateways: 1),
                notice: .rowProblem(row: "10.30.170.9", problem: .overlap(other: "10.30.170.0/24"))
            ),
            bypassSection(.noConfigFile, notice: .applyFailed(error: unreadableFile)),
            bypassSection(.configInvalid, notice: nil),
            bypassSection(.noNetwork, notice: nil),
            bypassSection(.failed(failure: .routesNotKept, retryInSeconds: 10, stoppedAfterRetries: nil), notice: nil),
            bypassSection(.failed(failure: .networkChanged, retryInSeconds: nil, stoppedAfterRetries: 3), notice: nil),
            bypassSection(
                .failed(
                    failure: .helperRefused(
                        message: "the route list has repeats, too many routes, or routes this helper does not write"
                    ),
                    retryInSeconds: nil, stoppedAfterRetries: nil
                ),
                notice: nil
            ),
            bypassSection(
                .failed(
                    failure: .settingsNotChanged(message: "** Error: The parameters were not valid."),
                    retryInSeconds: nil, stoppedAfterRetries: nil
                ),
                notice: nil
            ),
            bypassSection(
                .failed(failure: .importNotSaved(error: unreadableFile), retryInSeconds: nil, stoppedAfterRetries: nil),
                notice: nil
            ),
            bypassSection(
                .failed(
                    failure: .helperUnreachable(failure: .connectionLost), retryInSeconds: nil, stoppedAfterRetries: nil
                ),
                notice: nil
            ),
            bypassSection(
                .failed(
                    failure: .helperBuildMismatch(helperBuild: "20261108090000", appBuild: "20261109120000"),
                    retryInSeconds: nil, stoppedAfterRetries: nil
                ),
                notice: nil
            ),
        ]

        static let configSections: [ConfigSection] = [
            ConfigSection(
                file: ConfigFileState(
                    path: configPath, exists: true, loadError: unreadableFile,
                    warnings: [
                        ConfigWarning(kind: .unknownKey(path: "vpn.colour"), location: LineColumn(line: 3, column: 3)),
                        ConfigWarning(kind: .insecurePermissions(mode: 0o644), location: nil),
                    ]
                ),
                account: AccountFormState(
                    revision: 2, host: "https://vpn.example.com", port: "4433", username: "alice", realm: "staff",
                    gmailAddress: "alice@example.com", vpnPasswordEntered: true, appPasswordEntered: true,
                    vpnPasswordStored: false, appPasswordStored: true, savedAccountProblem: .invalidHost,
                    feedback: .saved, save: .enabled, revert: .enabled, importReport: nil,
                    defaultImportFile: "/Users/alice/.config/openfortivpn/config"
                ),
                options: ConnectionOptions(
                    autoConnectOnLaunch: false, sessionMaxMinutes: MinutesSetting(value: 180, minimum: 10, maximum: 1440),
                    proactiveReconnect: false, marginMinutes: MinutesSetting(value: 5, minimum: 1, maximum: 60),
                    reconnectAfterSleep: true, minSleepMinutes: MinutesSetting(value: 30, minimum: 1, maximum: 1440)
                ),
                optionsError: nil,
                openAtLogin: OpenAtLogin(available: false, enabled: false, requiresApproval: true, error: .notInstalledCopy),
                logFile: "/Users/alice/Library/Logs/FortiVPNManager/openfortivpn.log"
            ),
            configSection(
                accountForm(3, feedback: .failed(error: .invalidPort)),
                optionsError: .invalidSettings(
                    problem: ConfigProblem(kind: .marginNotLessThanSession(margin: 60, session: 60), location: nil)
                )
            ),
            configSection(accountForm(3, feedback: .failed(error: .passwordNeedsAccount))),
            configSection(accountForm(3, feedback: .failed(error: .appPasswordNeedsAddress))),
            configSection(accountForm(3, feedback: .failed(error: .accountValue(
                problem: .tooLong(field: .username, maxBytes: 64)
            )))),
            configSection(accountForm(3, feedback: .failed(error: .accountValue(
                problem: .leadingOrTrailingSpace(field: .host)
            )))),
            configSection(accountForm(3, feedback: .failed(error: .accountValue(
                problem: .disallowedCharacters(field: .realm)
            )))),
            configSection(accountForm(3, feedback: .failed(error: .invalidVpnPassword(maxBytes: 255)))),
            configSection(accountForm(3, feedback: .failed(error: .config(error: unreadableFile)))),
            configSection(accountForm(3, feedback: .failed(error: .secretStore(failure: storeFailure)))),
            configSection(accountForm(4, importReport: .imported(report: importReport(.saved)))),
            configSection(accountForm(4, importReport: .imported(report: importReport(.notSavedWithoutUsername)))),
            configSection(accountForm(4, importReport: .imported(report: importReport(.notSaved(failure: storeFailure))))),
            configSection(accountForm(4, importReport: .imported(report: importReport(nil)))),
            configSection(accountForm(4, importReport: .failed(error: .sourceNotFound(
                path: "/Users/alice/.config/openfortivpn/config"
            )))),
            configSection(accountForm(4, importReport: .failed(error: .unreadable(
                path: "/Users/alice/vpn.conf", problem: .notRegularFile
            )))),
            configSection(accountForm(4, importReport: .failed(error: .missingHost(path: "/Users/alice/vpn.conf")))),
            configSection(accountForm(4, importReport: .failed(error: .config(error: unreadableFile)))),
            configSection(accountForm(4, importReport: .failed(error: .noFortiClientConnection))),
            configSection(accountForm(4, importReport: .failed(error: .invalidFortiClientServer(connection: "Office")))),
        ]

        static let configFileErrors: [ConfigFileError] = [
            .invalidSettings(problem: ConfigProblem(
                kind: .scheduleEndNotAfterStart(start: TimeOfDay(hour: 18, minute: 0), end: TimeOfDay(hour: 9, minute: 0)),
                location: nil
            )),
            unreadableFile,
            .notRegularFile(path: configPath),
            .danglingSymlink(path: configPath),
            .keptChanging(path: configPath),
            .io(operation: "rename", path: configPath, errno: 13),
        ]

        static func problem(_ kind: ConfigProblemKind) -> ConfigProblem {
            ConfigProblem(kind: kind, location: LineColumn(line: 2, column: 9))
        }

        static let configProblems: [ConfigProblem] = [
            ConfigProblem(kind: .fileTooLarge(byteCount: 70_000, limit: 65_536), location: nil),
            problem(.invalidUtf8),
            problem(.disallowedCharacter(scalar: 0x85)),
            problem(.syntax(problem: "did not find expected key")),
            problem(.multipleDocuments),
            problem(.nestingTooDeep),
            problem(.duplicateKey(key: "port")),
            problem(.rootNotMapping),
            problem(.anchorOrAlias),
            problem(.mergeKey),
            problem(.explicitTag(tag: "!!str")),
            problem(.complexKey),
            problem(.typeMismatch(path: "vpn", expected: .mapping)),
            problem(.typeMismatch(path: "routes.networks", expected: .sequence)),
            problem(.typeMismatch(path: "vpn.host", expected: .string)),
            problem(.typeMismatch(path: "vpn.port", expected: .integer)),
            problem(.typeMismatch(path: "connection.auto_connect_on_launch", expected: .boolean)),
            problem(.invalidInteger(path: "vpn.port", raw: "0443")),
            problem(.invalidBoolean(path: "connection.auto_connect_on_launch", raw: "yes")),
            problem(.quotedNumber(path: "vpn.port", raw: "443")),
            problem(.quotedBoolean(path: "connection.proactive_reconnect.enabled", raw: "true")),
            problem(.invalidChoice(path: "otp.method", raw: "pop3", allowed: ["imap", "gmail_api"])),
            problem(.outOfRange(path: "vpn.port", raw: "70000", minimum: 1, maximum: 65535)),
            problem(.marginNotLessThanSession(margin: 30, session: 20)),
            problem(.invalidTime(path: "connection.schedule.start", raw: "9am")),
            problem(.scheduleEndNotAfterStart(start: TimeOfDay(hour: 18, minute: 0), end: TimeOfDay(hour: 9, minute: 0))),
            problem(.invalidCharacter(path: "vpn.username", scalar: 0x2028)),
            problem(.newerSchemaVersion(version: 2)),
            problem(.invalidRouteEntry(
                path: "routes.bypass_vpn", index: 1, raw: "10.30.170.5/24", problem: .hostBitsSet(network: "10.30.170.0/24")
            )),
            problem(.tooManyRouteEntries(path: "routes.networks", count: 101, limit: 100)),
            problem(.duplicateRouteEntry(path: "routes.bypass_vpn", index: 2)),
            problem(.overlappingRouteEntry(path: "routes.networks", index: 1, other: "10.20.0.0/16")),
        ]

        static let configWarnings: [ConfigWarning] = [
            ConfigWarning(kind: .unknownKey(path: "vpn.colour"), location: LineColumn(line: 4, column: 5)),
            ConfigWarning(kind: .invalidTrustedCert(index: 1), location: LineColumn(line: 4, column: 5)),
            ConfigWarning(kind: .duplicateTrustedCert(index: 2), location: LineColumn(line: 4, column: 5)),
            ConfigWarning(kind: .tooManyTrustedCerts(limit: 4), location: LineColumn(line: 4, column: 5)),
            ConfigWarning(kind: .duplicateRouteEntry(path: "routes.networks", index: 3), location: LineColumn(line: 4, column: 5)),
            ConfigWarning(kind: .multipleHardLinks(count: 2), location: nil),
            ConfigWarning(kind: .insecurePermissions(mode: 0o644), location: nil),
            ConfigWarning(kind: .permissionsCorrected(previousMode: 0o640), location: nil),
        ]

        static let portRequests: [FromRuntime] = [
            .portRequest(id: 1, request: .readSecret(item: .vpnPassword(username: "alice", host: "vpn.example.com"))),
            .portRequest(id: 2, request: .hasSecret(item: .gmailAppPassword(address: "alice@example.com"))),
            .portRequest(id: 3, request: .writeSecret(
                item: .googleRefreshToken(address: "Alice@Example.com"), value: "1//0g-refresh"
            )),
            .portRequest(id: 4, request: .deleteSecret(item: .googleClientSecret(
                clientId: "987-xyz.apps.googleusercontent.com"
            ))),
            .portRequest(id: 5, request: .helperRegistration),
            .portRequest(id: 6, request: .registerHelper),
            .portRequest(id: 7, request: .helperBuild),
            .portRequest(id: 8, request: .callHelper(call: .connect(
                json: #"{"attemptID":"\#(attempt)","host":"vpn.example.com","password":"p"}"#
            ))),
            .portRequest(id: 9, request: .callHelper(call: .submitOtp(attempt: attempt, code: "123456"))),
            .portRequest(id: 10, request: .callHelper(call: .disconnect(attempt: attempt, reason: "userDisconnect"))),
            .portRequest(id: 11, request: .callHelper(call: .disconnect(attempt: attempt, reason: "clientInvalidated"))),
            .portRequest(id: 12, request: .callHelper(call: .status)),
            .portRequest(id: 13, request: .callHelper(call: .exitWhenIdle)),
            .portRequest(id: 14, request: .callHelper(call: .readBypassRoutes)),
            .portRequest(id: 15, request: .callHelper(call: .writeBypassRoutes(json: #"{"expected":{},"routes":[]}"#))),
            .portRequest(id: 17, request: .routeInterface(destination: "203.0.113.10")),
            .portRequest(id: 18, request: .interfaceAddresses),
            .portRequest(id: 19, request: .primaryRouter),
            .portRequest(id: 20, request: .screenState),
            .portRequest(id: 21, request: .openBrowser(
                url: "https://accounts.google.com/o/oauth2/v2/auth?client_id=x&state=y"
            )),
            .portRequest(id: 22, request: .loginItemRegistration),
            .portRequest(id: 23, request: .setLoginItem(enabled: false)),
            .portRequest(id: 24, request: .fortiClientConnections),
        ]

        static let portActions: [FromRuntime] = [
            .portAction(.log(level: .info, category: .connection, message: "state: connected")),
            .portAction(.log(level: .notice, category: .readiness, message: "readiness: ready")),
            .portAction(.log(level: .error, category: .connection, message: "presence: away")),
            .portAction(.log(level: .info, category: .config, message: "config changed")),
            .portAction(.log(level: .info, category: .state, message: "state saved")),
            .portAction(.log(level: .info, category: .logfile, message: "log file rotated")),
            .portAction(.log(level: .info, category: .import, message: "imported")),
            .portAction(.log(level: .info, category: .oauth, message: "token refreshed")),
            .portAction(.log(level: .info, category: .otp, message: "code found")),
            .portAction(.log(level: .info, category: .bypassRoutes, message: "routes written")),
            .portAction(.log(level: .info, category: .loginItem, message: "registered")),
            .portAction(.postNotification(notification: .exceptionRulesNotApplied(retries: 3, failure: .routesNotKept))),
            .portAction(.holdActivity(hold: true)),
            .portAction(.openHelperApprovalSettings),
            .portAction(.approveQuit),
        ]

        // MARK: Swift-sent

        static let launched: [ToRuntime] = [
            .launched(facts: AppFacts(
                appBuild: "20261109120000", installedCopy: true,
                builtInOauthClient: #"{"installed":{"client_id":"123-abc.apps.googleusercontent.com"}}"#,
                timeZone: "Asia/Seoul",
                signInPages: SignInPages(
                    completed: "FortiVPNManager: you can close this window.",
                    notCompleted: "FortiVPNManager: Google sign-in was not completed. You can close this window.",
                    invalid: "FortiVPNManager: this sign-in response is not valid. Start the sign-in again in Settings."
                )
            )),
        ]

        static let commands: [ToRuntime] = [
            .command(.connect),
            .command(.disconnect),
            .command(.reconnectNow),
            .command(.settingsOpened),
            .command(.settingsClosed),
            .command(.editAccount(revision: 4, form: AccountForm(
                host: "vpn.example.com", port: "443", username: "alice", realm: "", gmailAddress: "alice@example.com",
                vpnPassword: "p@ss", appPassword: ""
            ))),
            .command(.saveAccount),
            .command(.revertAccount),
            .command(.importClientSettings(file: nil)),
            .command(.importClientSettings(file: "/Users/alice/vpn.conf")),
            .command(.dismissImportReport),
            .command(.setOtpMethod(method: .gmailApi)),
            .command(.setOtpMethod(method: .imap)),
            .command(.testGmail),
            .command(.connectGoogleAccount),
            .command(.cancelGoogleSignIn),
            .command(.disconnectGoogleAccount),
            .command(.importOAuthClient(file: "/Users/alice/Downloads/client_secret_123.json")),
            .command(.useBuiltInOAuthClient),
            .command(.setConnectionOption(option: .scheduleEnabled(enabled: true))),
            .command(.setConnectionOption(option: .scheduleDay(day: .mon, selected: false))),
            .command(.setConnectionOption(option: .scheduleDay(day: .sun, selected: true))),
            .command(.setConnectionOption(option: .awayMinutes(minutes: 45))),
            .command(.setConnectionOption(option: .autoConnectOnLaunch(enabled: false))),
            .command(.setConnectionOption(option: .sessionMaxMinutes(minutes: -5))),
            .command(.setConnectionOption(option: .proactiveReconnect(enabled: true))),
            .command(.setConnectionOption(option: .proactiveMarginMinutes(minutes: 10))),
            .command(.setConnectionOption(option: .reconnectAfterSleep(enabled: true))),
            .command(.setConnectionOption(option: .minSleepMinutes(minutes: 30))),
            .command(.setScheduleHours(start: TimeOfDay(hour: 9, minute: 0), end: TimeOfDay(hour: 18, minute: 30))),
            .command(.setOpenAtLogin(enabled: true)),
            .command(.registerHelper),
            .command(.openHelperApprovalSettings),
            .command(.setBypassRows(list: .networks, rows: ["10.20.161.0/24"])),
            .command(.setBypassRows(list: .targets, rows: ["10.30.161.16", ""])),
            .command(.addCurrentNetwork),
            .command(.discardBypassChanges),
            .command(.applyBypassRules),
        ]

        static let portResponses: [ToRuntime] = [
            .portResponse(id: 1, response: .secret(value: "p@ss")),
            .portResponse(id: 1, response: .secret(value: nil)),
            .portResponse(id: 2, response: .hasSecret(exists: true)),
            .portResponse(id: 3, response: .secretChanged),
            .portResponse(id: 4, response: .secretStoreFailed(failure: storeFailure)),
            .portResponse(id: 5, response: .registration(status: .notRegistered)),
            .portResponse(id: 5, response: .registration(status: .enabled)),
            .portResponse(id: 5, response: .registration(status: .requiresApproval)),
            .portResponse(id: 5, response: .registration(status: .notFound)),
            .portResponse(id: 6, response: .registered),
            .portResponse(id: 6, response: .registrationFailed(error: .notInstalledCopy)),
            .portResponse(id: 6, response: .registrationFailed(error: .failed(description: "Operation not permitted"))),
            .portResponse(id: 7, response: .helperBuild(build: "20261109120000")),
            .portResponse(id: 8, response: .helperReply(json: nil)),
            .portResponse(id: 12, response: .helperReply(json: #"{"childAlive":false}"#)),
            .portResponse(id: 13, response: .helperReplyFlag(value: true)),
            .portResponse(id: 8, response: .helperUnreachable(failure: .appNotSigned)),
            .portResponse(id: 8, response: .helperUnreachable(failure: .connection(
                description: "Couldn’t communicate with a helper application."
            ))),
            .portResponse(id: 8, response: .helperUnreachable(failure: .connectionLost)),
            .portResponse(id: 8, response: .helperUnreachable(failure: .noReply)),
            .portResponse(id: 7, response: .helperUnreachable(failure: .unreadableReply(
                description: "The data couldn’t be read because it is missing."
            ))),
            .portResponse(id: 8, response: .helperBuildMismatch(helperBuild: "20261108090000")),
            .portResponse(id: 17, response: .routeInterface(interface: "en0")),
            .portResponse(id: 17, response: .routeInterface(interface: nil)),
            .portResponse(id: 18, response: .interfaceAddresses(addresses: [
                InterfaceAddress(name: "en0", up: true, ipv4: "10.20.161.20"),
                InterfaceAddress(name: "ppp0", up: true, ipv4: nil),
            ])),
            .portResponse(id: 19, response: .primaryRouter(router: "10.20.161.1")),
            .portResponse(id: 19, response: .primaryRouter(router: nil)),
            .portResponse(id: 20, response: .screenState(locked: true, displaysAsleep: false)),
            .portResponse(id: 21, response: .browserOpened(opened: false)),
            .portResponse(id: 22, response: .fortiClientConnections(connections: [
                FortiClientConnection(name: "Office", server: "vpn.example.com:10443"),
            ])),
            .portResponse(id: 22, response: .fortiClientConnections(connections: [])),
        ]

        static let systemEvents: [ToRuntime] = [
            .event(.willSleep(at: at(t0))),
            .event(.didWake),
            .event(.screenLocked(at: at(t0 + 250))),
            .event(.screenUnlocked),
            .event(.displaysSlept(at: at(t0 + 500))),
            .event(.displaysWoke),
            .event(.networkPathChanged(satisfied: true, interfaces: [
                PathInterface(name: "en0", kind: .wiredEthernet),
                PathInterface(name: "en1", kind: .wifi),
                PathInterface(name: "pdp_ip0", kind: .cellular),
                PathInterface(name: "lo0", kind: .loopback),
                PathInterface(name: "ppp0", kind: .other),
            ])),
            .event(.networkPathChanged(satisfied: false, interfaces: [])),
            .event(.networkSettingsChanged),
            .event(.clockChanged),
            .event(.timeZoneChanged(timeZone: "Europe/Berlin")),
            .event(.appActivated),
            .event(.quitRequested),
            .event(.helperEvent(json: #"{"attemptID":"\#(attempt)","kind":{"milestone":{"_0":0}}}"#)),
            .event(.helperOutput(
                json: #"[{"date":1762412345.25,"source":"stdout","text":"INFO:   Connected to gateway."}]"#
            )),
            .event(.helperConnectionLost),
        ]
    }
}
