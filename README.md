# FortiVPNManager

openfortivpn으로 FortiGate SSL VPN에 자동 접속하고, 끊기면 자동으로 다시 접속하는 macOS 메뉴바 앱입니다.

- Gmail(Google Workspace) IMAP에서 FortiGate 이메일 OTP를 자동으로 읽어 입력합니다.
- 서버가 세션을 강제로 끊으면 즉시 재연결하고, 만료 직전에는 미리 재연결합니다.
- 설정은 `~/.config/FortiVPNManager/config.yaml`에, 비밀번호는 Keychain에 저장합니다.

설계 문서: [docs/DESIGN.md](docs/DESIGN.md)

## 요구 사항

- macOS 15 이상 (개발 환경: macOS 27)
- Xcode, [XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`)
- openfortivpn (`brew install openfortivpn`)
- 무료 Apple ID로 만든 **Apple Development** 서명 인증서 (유료 개발자 계정 불필요)

## 처음 한 번 준비

```sh
sudo xcode-select -s /Applications/Xcode.app
sudo xcodebuild -license accept
sudo xcodebuild -runFirstLaunch
brew install xcodegen
```

1. Xcode > Settings > Accounts에서 Apple ID를 추가합니다.
2. 같은 화면의 Manage Certificates… > `+` > **Apple Development**를 선택해 인증서를 만듭니다.
3. `make local-config`를 실행하면 인증서에서 Team ID를 읽어 `Config/Local.xcconfig`를 만듭니다.

## Google OAuth 클라이언트 (Gmail API용, 선택)

OTP 메일을 가장 빠르게 감지하는 Gmail API 방식은 Google Cloud의 **데스크톱 앱 OAuth 클라이언트**가 필요합니다.
클라이언트는 빌드할 때 앱에 내장되며, 사용자는 설정창에서 **Google 계정 연결**만 누르면 됩니다.

1. Google Cloud 프로젝트에서 Gmail API를 사용 설정하고, OAuth 동의 화면(조직 내부용이면 *Internal*)에
   `https://www.googleapis.com/auth/gmail.modify` 범위를 추가합니다.
2. **데스크톱 앱** 유형의 OAuth 클라이언트를 만들고 JSON을 내려받습니다.
3. 내려받은 JSON을 `Config/GoogleOAuthClient.json`으로 복사합니다. 이 파일은 `.gitignore`에 포함되어 있어
   저장소에 올라가지 않습니다(client secret 포함).

파일이 없으면 내장 클라이언트 없이 빌드되며, 설정창 계정 탭의 **OAuth 클라이언트 (고급)**에서 JSON을 직접
가져올 수 있습니다. 이 경우에도 앱 비밀번호(IMAP) 방식은 그대로 쓸 수 있습니다.

## 빌드와 설치

| 명령 | 내용 |
|---|---|
| `make test` | 패키지 단위 테스트 (`swift test`) |
| `make build` | Debug 빌드 (`.build/xcode`) |
| `make verify` | 서명/번들 구성 검사 |
| `make install` | Release 빌드를 `/Applications`에 설치하고 실행 |
| `make logs` | 앱/헬퍼 로그 실시간 보기 |

헬퍼(root 데몬)는 `/Applications`에 설치된 앱에서만 등록됩니다. 처음 실행하면 시스템 설정 > 일반 > 로그인 항목 및 확장 프로그램에서 승인해야 합니다.

## 파일 위치

| 내용 | 위치 |
|---|---|
| 설정 | `~/.config/FortiVPNManager/config.yaml` |
| 비밀번호, Google 로그인 토큰 | 로그인 Keychain |
| 앱 상태 (실패 횟수 등) | `~/Library/Application Support/FortiVPNManager/state.json` |
| openfortivpn 원본 로그 | `~/Library/Logs/FortiVPNManager/openfortivpn.log` |

## 구조

```
App/                    메뉴바 앱 (SwiftUI + AppKit)
Helper/                 root 헬퍼 (LaunchDaemon, XPC)
Packages/FortiCore/     순수 로직 패키지 (상태머신, 파서, IMAP, 설정, 프로세스 감독)
Support/LaunchDaemons/  헬퍼 launchd plist
```
