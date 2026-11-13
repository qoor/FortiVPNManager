# FortiVPNManager

[English](README.md) · 한국어

openfortivpn으로 FortiGate SSL VPN에 자동 접속하고, 끊기면 자동으로 다시 접속하는 macOS 메뉴바 앱입니다.

## 특징

- **네이티브 UI**: OS별 네이티브 UI로 높은 반응성과 OS에 자연스럽게 통합된 UX를 제공합니다.
- **크로스 플랫폼 지원 (예정)**: 다양한 OS와 VPN 백엔드를 지원합니다.
- **최소 설정**: VPN 서버 정보, OTP 인증 정보, Helper 구성만으로 VPN 자동 재연결이 동작합니다.
- **스케줄 관리**: 자동 재연결 스케줄을 요일별 시간대로 관리할 수 있습니다.
- **고속 연결**: 빠른 재연결과 빠른 OTP 자동 인증으로 재연결이 통신에 주는 영향을 최소화합니다.

## 주요 기능

- Gmail(Google Workspace)로 온 FortiGate 이메일 OTP를 자동으로 읽어 입력합니다. Gmail API(Google 계정 연결)를 쓰고, 안 되면 IMAP(앱 비밀번호)으로 전환합니다.
- 서버가 세션을 끊으면 잠시 뒤, 누군가 강제로 끝내면 곧바로 재연결하고, 만료 직전에는 미리 재연결합니다.
- 예외 규칙: 지정한 사내 대역에 연결돼 있을 때 예외 대상 호스트·대역은 VPN을 거치지 않고 그 네트워크로 바로 나갑니다(macOS 추가 라우트, 재부팅 후에도 유지). 다른 네트워크에서는 모두 VPN을 거칩니다.
- 설정은 `~/.config/FortiVPNManager/config.yaml`에, 비밀번호는 Keychain에 저장합니다.

문서: [설계](docs/DESIGN.md) · [기능 명세](docs/SPEC.md) · [설정 UI](docs/SETTINGS-UI.md)

## 요구 사항

- macOS 15 이상, Apple Silicon (개발 환경: macOS 27)
- Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- Rust ([rustup](https://rustup.rs)으로 설치, `~/.cargo/bin/cargo`를 씁니다)
- openfortivpn (`brew install openfortivpn`)
- 무료 Apple ID로 만든 **Apple Development** 서명 인증서 (유료 개발자 계정 불필요)

## 처음 한 번 준비

```sh
sudo xcode-select -s /Applications/Xcode.app
sudo xcodebuild -license accept
sudo xcodebuild -runFirstLaunch
brew install xcodegen
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

1. Xcode > Settings > Accounts에서 Apple ID를 추가합니다.
2. 같은 화면의 Manage Certificates… > `+` > **Apple Development**를 선택해 인증서를 만듭니다.
   인증서가 유효하지 않다고 나오면 Apple WWDR G3 중간 인증서를 로그인 Keychain에 추가합니다.
3. `make local-config`를 실행하면 인증서에서 Team ID를 읽어 `packaging/macos/Config/Local.xcconfig`를 만듭니다.

## Google OAuth 클라이언트 (Gmail API용, 선택)

OTP 메일을 가장 빠르게 감지하는 Gmail API 방식은 Google Cloud의 **데스크톱 앱 OAuth 클라이언트**가 필요합니다.
클라이언트는 빌드할 때 앱에 내장되며, 사용자는 설정창에서 **Google 계정 연결**만 누르면 됩니다.

1. Google Cloud 프로젝트에서 Gmail API를 사용 설정하고, OAuth 동의 화면(조직 내부용이면 *Internal*)에
   `https://www.googleapis.com/auth/gmail.modify` 범위를 추가합니다.
2. **데스크톱 앱** 유형의 OAuth 클라이언트를 만들고 JSON을 내려받습니다.
3. 내려받은 JSON을 `packaging/GoogleOAuthClient.json`으로 복사합니다. 이 파일은 `.gitignore`에 포함되어 있어
   저장소에 올라가지 않습니다(client secret 포함).

파일이 없으면 내장 클라이언트 없이 빌드되며, 설정창 계정 pane의 **OAuth 클라이언트 (고급)**에서 JSON을 직접
가져올 수 있습니다. 이 경우에도 앱 비밀번호(IMAP) 방식은 그대로 쓸 수 있습니다.

## 빌드와 설치

| 명령 | 내용 |
|---|---|
| `make test` | Rust 테스트 전체와 Swift 테스트 묶음(공통 계약, Helper, UI·OS 연동) |
| `make build` | Debug 빌드 (`.build/xcode`) |
| `make verify` | 서명/번들 구성 검사 |
| `make install` | Release 빌드를 `/Applications`에 설치하고 실행합니다. 설치 전에 VPN이 연결되어 있었는데 터널이 돌아오지 않으면 이전 앱을 되돌려 놓습니다 |
| `make logs` | 앱/Helper 로그 실시간 보기 |

Helper(root 데몬)는 `/Applications`에 설치된 앱에서만 등록됩니다. 처음 실행하면 시스템 설정 > 일반 > 로그인 항목 및 확장 프로그램에서 승인해야 합니다.

## 파일 위치

| 내용 | 위치 |
|---|---|
| 설정 | `~/.config/FortiVPNManager/config.yaml` |
| 비밀번호, Google 로그인 토큰 | 로그인 Keychain |
| 앱 상태 (실패 횟수 등) | `~/Library/Application Support/FortiVPNManager/state.json` |
| openfortivpn 원본 로그 | `~/Library/Logs/FortiVPNManager/openfortivpn.log` |

## 구조

공통 로직은 Rust, macOS의 UI·OS 연동·OS 연동 Helper는 Swift입니다. 구성 요소와 역할은 [설계 §4](docs/DESIGN.md#4-구성과-역할)를 보세요.
