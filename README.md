# FortiVPNManager

English · [한국어](README.ko.md)

A macOS menu bar app that connects to FortiGate SSL VPN through openfortivpn and reconnects automatically when the connection drops.

## Features

- **Native UI**: A native UI on each OS delivers high responsiveness and a UX that fits naturally into the OS.
- **Cross-platform (planned)**: Support for multiple operating systems and VPN backends.
- **Minimal setup**: Automatic VPN reconnection needs only the VPN server details, the OTP authentication details, and the Helper setup.
- **Schedules**: Manage when automatic reconnection runs, with time ranges for each day of the week.
- **Fast connection**: Fast reconnection and fast automatic OTP authentication keep the impact of reconnects on your traffic to a minimum.

## What it does

- Reads the FortiGate email OTP sent to Gmail (Google Workspace) and enters it for you. It uses the Gmail API (Google account connection) and falls back to IMAP (app password) when that doesn't work.
- Reconnects shortly after the server ends the session, immediately when the session is forcibly terminated, and ahead of time just before it expires.
- Exception Rules: while connected to a specified corporate network range, the listed hosts and ranges bypass the VPN and go out directly through that network (macOS additional routes, kept across reboots). On any other network, everything goes through the VPN.
- Settings are stored in `~/.config/FortiVPNManager/config.yaml` and passwords in the Keychain.

Documents (Korean): [Design](docs/DESIGN.md) · [Specification](docs/SPEC.md)

## Requirements

- macOS 15 or later, Apple Silicon (developed on macOS 27)
- Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- Rust (install with [rustup](https://rustup.rs); the build uses `~/.cargo/bin/cargo`)
- openfortivpn (`brew install openfortivpn`)
- An **Apple Development** signing certificate created with a free Apple ID (no paid developer account required)

## One-time setup

```sh
sudo xcode-select -s /Applications/Xcode.app
sudo xcodebuild -license accept
sudo xcodebuild -runFirstLaunch
brew install xcodegen
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

1. Add your Apple ID in Xcode > Settings > Accounts.
2. On the same screen, choose Manage Certificates… > `+` > **Apple Development** to create a certificate.
   If the certificate shows as not valid, add the Apple WWDR G3 intermediate certificate to the login Keychain.
3. Run `make local-config`. It reads the Team ID from the certificate and creates `packaging/macos/Config/Local.xcconfig`.

## Google OAuth client (for the Gmail API, optional)

The Gmail API method, which detects OTP emails the fastest, needs a **Desktop app OAuth client** from Google Cloud.
The client is embedded in the app at build time, so users only need to click **Connect Google Account…** in Settings.

1. In a Google Cloud project, enable the Gmail API and add the `https://www.googleapis.com/auth/gmail.modify` scope
   to the OAuth consent screen (*Internal* if it's only for your organization).
2. Create an OAuth client of type **Desktop app** and download its JSON.
3. Copy the downloaded JSON to `packaging/macos/Config/GoogleOAuthClient.json`. This file is listed in `.gitignore`,
   so it never gets committed (it contains the client secret).

Without the file, the app builds without a built-in client, and you can import the JSON yourself under
**OAuth client (advanced)** in the Account pane of Settings. The app password (IMAP) method still works either way.

## Build and install

| Command | What it does |
|---|---|
| `make test` | All Rust tests plus the Swift test bundles (shared contract, Helper, UI and OS integration) |
| `make build` | Debug build (`.build/xcode`) |
| `make verify` | Checks signing and bundle layout |
| `make install` | Builds Release, installs it to `/Applications`, and launches it. If the VPN was connected before the install and the tunnel doesn't come back, it restores the previous app |
| `make logs` | Streams app and Helper logs live |

The Helper (root daemon) is registered only from an app installed in `/Applications`. On first launch, approve it in System Settings > General > Login Items & Extensions.

## File locations

| What | Where |
|---|---|
| Settings | `~/.config/FortiVPNManager/config.yaml` |
| Passwords, Google sign-in tokens | Login Keychain |
| App state (failure counts, etc.) | `~/Library/Application Support/FortiVPNManager/state.json` |
| Raw openfortivpn log | `~/Library/Logs/FortiVPNManager/openfortivpn.log` |

## Architecture

Shared logic is written in Rust; the macOS UI, OS integration, and the OS integration Helper are written in Swift. For the components and their roles, see [Design §4](docs/DESIGN.md#4-구성과-역할) (Korean).
