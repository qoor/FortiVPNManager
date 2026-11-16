# FortiVPNManager

[English](README.md) · 한국어

FortiGate SSL VPN 자동 재접속 관리 앱입니다.

## 특징

- **네이티브 UI**: OS별 네이티브 UI로 높은 반응성과 자연스러운 UX 제공
- **크로스 플랫폼**: macOS, Windows 지원
- **최소 설정**: VPN 서버 정보, OTP 인증 정보, Helper 구성만으로 자동 재연결 동작
- **스케줄 관리**: 요일별 시간대로 자동 재연결 스케줄 관리
- **고속 연결**: 빠른 재연결과 OTP 자동 인증으로 통신 영향 최소화
- **VPN 백엔드**: OpenConnect, openfortivpn 지원

## 주요 기능

- **OTP 인증**: Gmail 연동 OTP 자동 인증 지원
- **자동 재연결**: VPN 연결 끊김 시 자동 재연결 지원
- **예외 규칙**: VPN을 거치지 않고 기존 네트워크 경로로 통신할 네트워크 주소 및 대역 설정 지원
- **설정**: YAML 기반 설정 파일 지원
- **계정 관리**: OS 인증 정보 저장소(macOS Keychain / Windows Credential Manager)를 통해 안전하게 인증 정보 관리

문서: [설계](docs/DESIGN.md) · [기능 명세](docs/SPEC.md) · [설정 UI](docs/SETTINGS-UI.md)

## Google OAuth 클라이언트 (선택)

Gmail API 방식 OTP 인증에 쓰는 Google Cloud **데스크톱 앱 OAuth 클라이언트**를 빌드할 때 앱에 내장합니다.

1. Google Cloud 프로젝트에서 Gmail API를 사용 설정하고, OAuth 동의 화면에 `https://www.googleapis.com/auth/gmail.modify` 범위를 추가합니다.
2. **데스크톱 앱** 유형의 OAuth 클라이언트를 만들고 JSON을 내려받습니다.
3. JSON을 `packaging/GoogleOAuthClient.json`으로 복사합니다(저장소 제외 파일).

파일이 없으면 설정창에서 JSON을 직접 가져오거나 앱 비밀번호(IMAP) 방식을 씁니다. GitHub Release의 MSI에는 저장소 비밀값(`GOOGLE_OAUTH_CLIENT_JSON`)의 클라이언트가 내장됩니다.

## 버전과 릴리스

- **버전**: `연도.기능.수정`(예: `26.0.0`). 연도는 릴리스하는 해의 끝 두 자리이고, 모든 플랫폼이 저장소 맨 위 `VERSION` 파일의 번호를 씁니다.
- **릴리스**: `VERSION`을 고쳐 commit한 뒤 `v<버전>` tag를 push하면 Windows와 macOS 설치 패키지가 함께 GitHub Release로 올라갑니다. macOS 앱은 저장소 비밀값의 서명 인증서(`MACOS_SIGNING_CERTIFICATE`, `MACOS_SIGNING_CERTIFICATE_PASSWORD`)로 서명합니다.

## macOS

### 요구 사항

- macOS 15 이상, Apple Silicon
- Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- Rust ([rustup](https://rustup.rs)으로 설치)
- openfortivpn (`brew install openfortivpn`)
- 무료 Apple ID로 만든 **Apple Development** 서명 인증서 (유료 개발자 계정 불필요)

### 처음 한 번 준비

```sh
sudo xcode-select -s /Applications/Xcode.app
sudo xcodebuild -license accept
sudo xcodebuild -runFirstLaunch
brew install xcodegen
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

1. Xcode > Settings > Accounts에서 Apple ID를 추가합니다.
2. 같은 화면의 Manage Certificates… > `+` > **Apple Development**로 인증서를 만듭니다.
   - 인증서가 유효하지 않으면 Apple WWDR G3 중간 인증서를 로그인 Keychain에 추가합니다.
3. `make local-config`로 `packaging/macos/Config/Local.xcconfig`를 만듭니다.

### 빌드와 설치

| 명령 | 내용 |
|---|---|
| `make test` | Rust·Swift 테스트 |
| `make build` | Debug 빌드 (`.build/xcode`) |
| `make verify` | 서명·번들 구성 검사 |
| `make install` | Release 빌드 설치 및 실행 (`/Applications`) |
| `make pkg` | Release 빌드로 설치 패키지(`.build/FortiVPNManager-<버전>.pkg`) 생성 |
| `make logs` | 앱·Helper 로그 실시간 보기 |

- **설치**: GitHub Release의 pkg 실행. 서명하지 않은 pkg라 처음에는 macOS가 막으므로, 시스템 설정 > 개인정보 보호 및 보안에서 "그래도 열기"를 누른 뒤 설치합니다. openfortivpn은 Homebrew로 따로 설치합니다.
- **Helper**: `/Applications`에 설치된 앱에서만 등록, 첫 실행 시 시스템 설정 > 일반 > 로그인 항목 및 확장 프로그램에서 승인 필요

### 파일 위치

| 내용 | 위치 |
|---|---|
| 설정 | `~/.config/FortiVPNManager/config.yaml` |
| 비밀번호, Google 로그인 토큰 | 로그인 Keychain |
| 앱 상태 (실패 횟수 등) | `~/Library/Application Support/FortiVPNManager/state.json` |
| openfortivpn 원본 로그 | `~/Library/Logs/FortiVPNManager/openfortivpn.log` |

## Windows

### 요구 사항

- Windows 11 이상, x64
- Rust ([rustup](https://rustup.rs)으로 설치)
- .NET 10 SDK
- Visual Studio Build Tools의 **C++를 사용한 데스크톱 개발** 워크로드

### 빌드와 설치

| 명령 | 내용 |
|---|---|
| `packaging\windows\build.ps1 -Build <빌드 번호> -RustOutput <폴더>` | 미리 빌드한 Rust 결과물로 Release 빌드 및 설치 패키지(`.build\windows\FortiVPNManager-<버전>.msi`) 생성 |
| `scripts/msi-windows.sh <폴더>` | Mac에서 Rust를 빌드하고 Windows PC(ssh)에서 설치 패키지 생성 |

- **설치**: GitHub Release 또는 직접 생성한 MSI 실행

### 파일 위치

| 내용 | 위치 |
|---|---|
| 설정 | `%USERPROFILE%\.config\FortiVPNManager\config.yaml` |
| 비밀번호, Google 로그인 토큰 | Credential Manager |
| 앱 상태 (실패 횟수 등) | `%LOCALAPPDATA%\FortiVPNManager\state.json` |
| OpenConnect 원본 로그 | `%LOCALAPPDATA%\FortiVPNManager\Logs\openconnect.log` |

## 구조

- **공통 로직**: Rust
- **macOS**: UI, OS 연동, Helper OS 영역 (Swift)
- **Windows**: UI, OS 연동 (C#, WinUI 3), Helper OS 영역 (Rust, Windows 서비스)

구성 요소와 역할은 [설계 §4](docs/DESIGN.md#4-구성과-역할)를 보세요.
