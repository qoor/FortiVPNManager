# FortiVPNManager

English · [한국어](README.ko.md)

An app that keeps a FortiGate SSL VPN connected by reconnecting automatically.

## Features

- **Native UI**: A native UI on each OS, for high responsiveness and a natural UX
- **Cross-platform**: macOS and Windows
- **Minimal setup**: Automatic reconnection with only the VPN server details, the OTP details, and the Helper setup
- **Schedules**: Automatic reconnection scheduled by time ranges for each day of the week
- **Fast connection**: Fast reconnection and automatic OTP entry keep the impact on your traffic to a minimum
- **VPN backends**: OpenConnect and openfortivpn

## What it does

- **OTP authentication**: Automatic OTP authentication through Gmail
- **Automatic reconnection**: Reconnects when the VPN connection drops
- **Exception Rules**: Network addresses and ranges that bypass the VPN and use the existing network route
- **Settings**: A YAML settings file
- **Account management**: Credentials kept safely in the OS credential store (macOS Keychain / Windows Credential Manager)

Documents (Korean): [Design](docs/DESIGN.md) · [Specification](docs/SPEC.md) · [Settings UI](docs/SETTINGS-UI.md)

## Google OAuth client (optional)

A Google Cloud **Desktop app OAuth client** for OTP authentication through the Gmail API, embedded in the app at build time.

1. In a Google Cloud project, enable the Gmail API and add the `https://www.googleapis.com/auth/gmail.modify` scope to the OAuth consent screen.
2. Create an OAuth client of type **Desktop app** and download its JSON.
3. Copy the JSON to `packaging/GoogleOAuthClient.json` (kept out of the repository).

Without the file, import the JSON in Settings, or use the app password (IMAP) method. The MSI in GitHub Releases has the client from the repository secret `GOOGLE_OAUTH_CLIENT_JSON` built in.

## Versions and releases

- **Version**: `year.feature.fix` (e.g. `26.0.0`). The year is the last two digits of the release's calendar year, and every platform takes the number in `VERSION` at the repository root.
- **Release**: change `VERSION`, commit, and push a `v<version>` tag; the Windows installer goes up as a GitHub Release.

## macOS

### Requirements

- macOS 15 or later, Apple Silicon
- Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- Rust (install with [rustup](https://rustup.rs))
- openfortivpn (`brew install openfortivpn`)
- An **Apple Development** signing certificate created with a free Apple ID (no paid developer account required)

### One-time setup

```sh
sudo xcode-select -s /Applications/Xcode.app
sudo xcodebuild -license accept
sudo xcodebuild -runFirstLaunch
brew install xcodegen
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

1. Add your Apple ID in Xcode > Settings > Accounts.
2. On the same screen, create a certificate with Manage Certificates… > `+` > **Apple Development**.
   - If it shows as not valid, add the Apple WWDR G3 intermediate certificate to the login Keychain.
3. Run `make local-config` to create `packaging/macos/Config/Local.xcconfig`.

### Build and install

| Command | What it does |
|---|---|
| `make test` | Rust and Swift tests |
| `make build` | Debug build (`.build/xcode`) |
| `make verify` | Signing and bundle layout check |
| `make install` | Release build, installed and launched (`/Applications`) |
| `make logs` | Live app and Helper logs |

- **Helper**: registered only from an app installed in `/Applications`; approve it on first launch in System Settings > General > Login Items & Extensions

### File locations

| What | Where |
|---|---|
| Settings | `~/.config/FortiVPNManager/config.yaml` |
| Passwords, Google sign-in tokens | Login Keychain |
| App state (failure counts, etc.) | `~/Library/Application Support/FortiVPNManager/state.json` |
| Raw openfortivpn log | `~/Library/Logs/FortiVPNManager/openfortivpn.log` |

## Windows

### Requirements

- Windows 11 or later, x64
- Rust (install with [rustup](https://rustup.rs))
- .NET 10 SDK
- The **Desktop development with C++** workload of Visual Studio Build Tools

### Build and install

| Command | What it does |
|---|---|
| `packaging\windows\build.ps1 -Build <build number> -RustOutput <folder>` | Release build and installer (`.build\windows\FortiVPNManager-<version>.msi`) from prebuilt Rust output |
| `scripts/msi-windows.sh <folder>` | Rust built on a Mac, installer made on a Windows PC (over ssh) |

- **Install**: run the MSI from a GitHub Release or your own build

### File locations

| What | Where |
|---|---|
| Settings | `%USERPROFILE%\.config\FortiVPNManager\config.yaml` |
| Passwords, Google sign-in tokens | Credential Manager |
| App state (failure counts, etc.) | `%LOCALAPPDATA%\FortiVPNManager\state.json` |
| Raw OpenConnect log | `%LOCALAPPDATA%\FortiVPNManager\Logs\openconnect.log` |

## Architecture

- **Shared logic**: Rust
- **macOS**: UI, OS integration, OS integration Helper (Swift)
- **Windows**: UI, OS integration (C#, WinUI 3), Helper service (Rust)

For the components and their roles, see [Design §4](docs/DESIGN.md#4-구성과-역할) (Korean).
