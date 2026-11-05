# FortiVPNManager 설계

이 문서는 앱이 **왜 이렇게 만들어졌는지**를 기록한다. 배경, 목표, 핵심 결정과 그 이유, 구성 요소의 역할, 확인된 외부 제약, 결정 이력을 다룬다.
값, 시간, 필드, 파일 구성처럼 바뀔 수 있는 세부 사항은 코드가 기준이다. 이 문서에 다시 적지 않는다.

## 1. 배경
- FortiGate SSL VPN 서버가 약 3시간마다 세션을 강제로 끊는다. 그때마다 `sudo openfortivpn`을 다시 실행하고 Gmail로 온 이메일 OTP를 손으로 입력해야 했다.
- 개인 Mac에서 쓰는 도구다. 지인 배포는 추후 과제이고, 유료 Apple Developer 계정은 없다.

## 2. 목표와 범위
- **목표**
  - 메뉴바에 상주하면서 openfortivpn 실행, OTP 입력, 재연결을 사람 대신 한다.
  - 사용자가 신경 쓸 일을 최소로 줄인다. 인증서 확인 창, 수동 OTP 입력 창, 기술적인 설정 항목을 두지 않는다.
- **범위 밖**
  - 앱이 비정상 종료된 뒤 실행 중인 터널을 다시 이어받는 기능(adopt)
  - split DNS, 다중 VPN 프로필
  - 보안 강화 옵션(§8)
  - 공증된 배포본

## 3. 핵심 결정
| 결정 | 이유 |
|---|---|
| VPN 클라이언트를 직접 만들지 않고 Homebrew openfortivpn을 감싼다 | 이미 이 게이트웨이에서 동작이 확인된 도구다. Homebrew 바이너리를 root로 실행하는 위험(§8)은 개인 Mac이라 감수한다 |
| root 작업은 SMAppService.daemon으로 등록한 Helper가 맡고, 앱과 XPC로 통신한다 | openfortivpn(pppd, routing table 변경)은 root가 필요하다. 매번 관리자 암호를 묻지 않으면서 root 권한이 필요한 범위만 좁게 분리하기 위해서다 |
| 무료 Apple ID(Personal Team)의 Apple Development 인증서로 서명한다 | launchd는 등록한 프로그램을 cdhash가 아니라 Team ID와 signing identifier로 식별한다. 그래서 재빌드해도 다시 승인받을 필요가 없다. 그 대가로 **번들 ID(`com.qoor.FortiVPNManager`, `…Helper`)는 최초 등록 후 바꾸지 않는다** |
| 연결할 때마다 openfortivpn 프로세스를 새로 띄운다 | `--persistent`는 이메일 OTP 경로에서 이미 쓴 OTP를 다시 보낸다 |
| 비밀번호와 OTP는 stdin pipe로만 전달한다 | argv, 설정 파일, 환경 변수, 로그 어디에도 비밀값을 남기지 않는다. PTY를 쓰지 않고 기본 verbosity로 고정하면 openfortivpn 원본 로그에도 비밀값이 나오지 않는다 |
| 종료 원인은 openfortivpn 출력이 아니라 Helper가 기록한 사실로 판단한다 | openfortivpn의 exit code와 pppd 메시지만으로는 원인을 구분할 수 없다(§6). 대신 어느 단계까지 진행했는지, 누가 종료시켰는지, 터널이 얼마나 유지됐는지를 본다 |
| 서버 인증서는 무조건 자동 신뢰한다 | 사용자가 인증서를 신경 쓰지 않기를 원했다. openfortivpn이 인증서를 거부하면서 출력하는 SHA-256 digest를 config.yaml `vpn.trusted_certs`에 저장하고, 이후 연결부터는 이 값을 넘긴다. 그래서 평소에는 openfortivpn을 한 번만 실행한다. 거부는 비밀번호를 보내기 전에 일어나므로 계정 잠금과는 무관하다 |
| OTP는 Gmail API(OAuth)로 읽고, 실패하면 IMAP IDLE(앱 비밀번호)로 전환한다 | IMAP은 메일이 Gmail에 도착한 뒤 IMAP에서 보이기까지 약 7초가 늦었다. Gmail API는 도착 즉시 감지한다. API 인증이 실패하고 앱 비밀번호가 있으면 같은 시도 안에서 IMAP으로 넘어간다 |
| OAuth Desktop client는 빌드할 때 앱에 넣는다 | client는 사용자가 아니라 앱을 식별하는 값이므로 사용자가 직접 가져올 이유가 없다. Desktop client의 secret은 비밀로 취급되지 않고 PKCE로 보호된다. 다른 client를 쓰고 싶으면 계정 탭의 고급 항목에서 가져와 덮어쓸 수 있다 |
| 사용한 OTP 메일은 휴지통으로 옮긴다 | 다음 시도에서 이전 코드를 잘못 고르지 않게 하고, 받은편지함에 OTP 메일이 쌓이지 않게 한다 |
| 수동 OTP 입력 창은 만들지 않는다 | 자동화가 목적이다. OTP를 읽지 못하면 실패로 처리한다 |
| 인증 실패는 보수적으로 센다 | FortiGate는 같은 IP에서 2회 실패하면 일정 시간 차단한다. 그래서 단순한 통신 끊김일 가능성이 있어도 로그인 단계에서 끝난 시도는 인증 실패로 센다. 두 번째 실패에서는 멈추고 사용자의 [연결]을 기다린다. 네트워크 오류는 세지 않고 backoff한다 |
| 세션 만료 전에 미리 재연결한다(사전 재연결) | 작업 도중 갑자기 끊기는 대신, 정해 둔 시각에 기존 세션을 먼저 끊고 새로 연결한다(break-before-make) |
| 잠자기 중에는 터널을 유지하고, 깨어나면 상태를 보고 판단한다 | 짧은 잠자기마다 재연결해서 OTP 메일이 오는 일을 막는다 |
| health check는 프로세스와 ppp interface만 본다 | Apple pppd에는 LCP echo가 없고 openfortivpn에는 network timeout이 없다. 그래서 별도 ping 대상을 두지 않는 대신 연결 단계마다 watchdog을 둔다 |
| 설정은 config.yaml, 비밀번호는 Keychain, 기기별 일시 상태는 state.json에 둔다 | config.yaml은 사람이 편집할 수 있고 dotfiles로 옮길 수 있다. 로그인 시 열기 여부는 macOS(SMAppService)가 소유하는 상태라서 YAML에 두지 않는다 |
| config.yaml은 엄격하게 읽는다 | 사람이 직접 편집하는 파일이다. YAML 1.1의 8진수·60진수 해석, anchor/alias처럼 의도와 다르게 읽힐 수 있는 문법은 거부한다. 파일에 오류가 있으면 마지막 정상 설정을 유지하고 연결을 막는다 |
| 앱이 config.yaml을 다시 쓰면 직접 단 주석은 사라진다 | 사용자가 수락한 한계다. 앱이 파일을 쓰는 경우는 설정창 저장, 새 인증서 digest 기록, 예외 규칙 가져오기뿐이다 |
| 로그는 마스킹하지 않은 원본을 보여준다 | 기본 verbosity에서는 비밀값이 출력되지 않음을 openfortivpn 소스에서 확인했다. 문제를 추적할 때는 원본이 가장 유용하다 |
| 언어 | 코드, 주석, 앱과 Helper가 남기는 로그는 영어로 쓴다. UI 한국어는 String Catalog로 번역한다. UI에서 "헬퍼"가 아니라 "Helper"라고 쓴다 |

## 4. 구성과 역할

### 4.1 앱과 Helper
- **앱** (사용자 권한, 메뉴바 상주)
  - 모든 판단을 맡는다. 연결 state machine, OTP 처리, 재연결 스케줄, config.yaml·Keychain·state.json, 서버 인증서 digest, 예외 규칙 계산, UI, 로그 보관이 여기에 속한다.
- **Helper** (root LaunchDaemon, 요청이 있을 때 launchd가 실행)
  - 실행하고 감시하는 일만 한다. openfortivpn 실행, stdin으로 비밀번호·OTP 주입, prompt 감지, 단계별 watchdog, 프로세스 종료, 비정상 종료 뒤 routing table 복구, 남은 프로세스 정리, 네트워크 서비스의 additional routes 읽기·쓰기가 여기에 속한다.
  - 상태를 저장하지 않고 config.yaml도 읽지 않는다. 앱이 보낸 값 중 검증을 통과한 것만 쓴다. 환경 변수는 PATH만 넘긴다(openfortivpn이 proxy 환경 변수를 읽기 때문, §6).
- **둘을 나눈 이유**: root로 실행되는 코드를 최소로 줄이기 위해서다. 앱은 root 권한 없이 자유롭게 재시작하거나 재설치할 수 있다.

### 4.2 앱과 Helper 사이의 규칙
- XPC 연결은 양쪽이 서로의 code signing requirement(Team ID + signing identifier)를 확인한다. 비밀번호는 Helper의 버전을 확인한 뒤에만 보낸다.
- 앱과 Helper는 같은 빌드여야 한다(BuildID 비교). 다르면 연결하지 않고 "Helper 버전 불일치"로 표시한다. 그래서 둘 사이의 메시지 형식은 구버전 호환을 고려하지 않는다.
- Helper는 앱의 연결이 끊기면(종료, 강제 종료, 크래시) 터널을 정상 종료한다. 그래서 세 경우가 모두 같은 경로를 탄다.
- Helper가 보내는 이벤트와 로그 출력은 하나의 순서 있는 채널로 전달된다. 연결 해제나 종료가 끝났는지는 요청에 대한 응답이 아니라 Helper가 보내는 종료 이벤트로 판단한다.

### 4.3 코드 구성
- `App/`: 메뉴바 UI, 설정창, 연결 제어, OTP 세션, 시스템 감시
- `Helper/`: root 데몬
- `Packages/FortiCore/`: 시스템에 의존하지 않는 로직
  - `FortiShared`: 앱과 Helper가 공유하는 XPC 계약
  - `FortiCore`: 연결 state machine, openfortivpn 출력 해석, OTP 메일 판별, IMAP, 예외 규칙 계산
  - `FortiConfig`: config.yaml 모델과 읽기·쓰기. Yams에 의존하므로 앱만 쓴다
  - `FortiHelperCore`: Helper의 프로세스 감독

## 5. 동작 원칙

### 5.1 연결
1. 연결 전에 준비 상태를 확인한다. config.yaml이 유효한지, Keychain에 VPN 비밀번호와 Gmail 인증(앱 비밀번호 또는 Google 계정 연결)이 있는지, Helper가 승인돼 있고 같은 빌드인지 본다. 부족하면 연결하지 않고 무엇이 필요한지 보여준다.
2. Gmail 세션을 먼저 열고, 그 시점 이후에 도착한 메일만 OTP 후보로 본다.
3. Helper가 openfortivpn을 실행하고 비밀번호를 넣는다. OTP prompt가 나오면 앱이 찾은 코드를 넣는다.
4. 터널이 올라오면 연결 상태가 되고, 사전 재연결 시각을 잡는다.
- VPN 계정(host, username)이 바뀌면 Keychain 항목도 새로 필요하다. 예전 비밀번호로 다른 계정에 로그인을 시도해서 잠기는 일을 막기 위해서다.
- 서버 주소나 포트가 바뀌면 저장된 인증서 digest를 비운다. 다음 연결에서 한 번만 다시 수집한다.

### 5.2 종료와 재시도
- 앱이 요청해서 끝난 연결(연결 해제, 사전 재연결, 잠자기, 종료)은 분류하지 않는다.
- 나머지는 아래 순서로 판단한다.
  1. OTP를 쓰기 전에 인증서가 거부됐다: digest를 저장하고 즉시 한 번 다시 시도한다. 실패로 세지 않는다.
  2. 안정적으로 유지되던 터널이 우리 요청 없이 끝났다: 서버가 세션을 끝낸 것으로 보고 곧바로 재연결한다.
  3. 게이트웨이에 닿기 전에 끝났다: 네트워크 오류로 보고 backoff한다.
  4. 로그인 단계에서 끝났다: 인증 실패로 센다. 단, 시도 중 네트워크가 바뀌었거나 깨어난 직후라면 네트워크 오류로 본다.
  5. OTP 미수신, OTP 거부, 인증 후 터널 실패, 너무 빨리 끊김: 인증 실패로 센다.
- 인증 실패가 1회면 잠금이 풀린 뒤 한 번 더 시도하고, 2회면 멈춘다. 일정 시간 안의 시도 횟수에도 상한을 둔다.
- Gmail 인증 자체가 실패하면 openfortivpn을 실행하지 않고 멈춘다. 잘못된 Gmail 설정으로 VPN 로그인 시도를 낭비하지 않기 위해서다.
- 멈춘 상태는 사용자가 [연결] 또는 [지금 재연결]을 눌러야 풀린다. 실패 횟수는 state.json에 저장되므로 앱을 다시 실행해도 유지된다.

### 5.3 잠자기와 깨어남
- 연결 중이거나 재시도를 기다리는 중에 잠들면 시도를 중단한다. 깨어나 네트워크가 돌아오면 새로 시도한다.
- 연결된 상태에서 깨어나면 다음 경우에만 재연결한다. 사전 재연결 시각이 지났을 때, 프로세스나 ppp interface가 사라졌을 때, 게이트웨이까지의 물리 경로(underlay)가 바뀌었을 때, 사용자가 켜 둔 "잠자기 후 재연결" 조건을 만족할 때다. 그 밖에는 유지한다.

### 5.4 스케줄
사용자가 요청한 기능이다. 퇴근 후나 주말에 OTP 메일 알림이 오는 것을 막고, 다른 기기에서 연 세션과 자동 재연결이 서로 다투지 않게 한다.
- 선택한 요일의 시작~종료 시각 하나를 스케줄 시간으로 본다. 시작 시각에는 [연결]을 누른 것처럼 동작한다.
- **스케줄 때문에 연결을 끊는 일은 없다.** 종료 시각 이후에는 자동 재연결만 제한한다.
  - 사용자가 자리에 있으면 이전처럼 재연결한다.
  - 자리를 비웠으면(화면 잠금, 디스플레이 잠자기, 시스템 잠자기) 재연결을 보류하고, 돌아오면 그때 재연결한다.
  - 자리 비움이 길어지면 재연결을 멈춘다. 살아 있는 터널은 그대로 두고, 그 세션이 끝나면 다시 붙지 않는다. [연결], [지금 재연결] 또는 다음 시작 시각에 풀린다.
- 직접 [연결]한 세션도 같은 규칙을 따른다.
- 앱 시작 시 자동 연결은 스케줄 시간 안일 때만 한다. 스케줄을 바꿔도 즉시 연결하거나 끊지 않는다.

### 5.5 예외 규칙
사내망에 직접 연결돼 있을 때, VPN이 가져가는 대역 중 지정한 host나 대역을 그 네트워크의 router로 바로 보낸다. VPN을 재연결해도 SSH 같은 TCP 세션이 끊기지 않게 하려는 것이다.
- **방식**: 주 네트워크 서비스의 additional routes(`networksetup -setadditionalroutes`)를 쓴다. macOS가 서비스 설정에 저장하고 서비스가 켜질 때마다 넣으므로, 재부팅과 VPN 재연결 뒤에도 유지되고 VPN 상태와 무관하게 동작한다. 쓰기에는 root가 필요해서 Helper가 실행한다.
- **source of truth는 config.yaml**이다. 포맷한 뒤에도 config.yaml만 복원하면 그대로 적용된다.
- **적용 대역(`routes.networks`)**: 이 Mac의 IPv4 주소가 목록의 대역 안에 있을 때만 적용한다. 밖에서는 앱이 넣은 route를 지워서 모든 트래픽이 VPN을 탄다. 이 목록에는 앱이 스스로 추가하지 않는다.
- **동기화 원칙**
  - 앱이 마지막으로 쓴 route를 서비스별로 state.json에 기억해 두고, 이를 기준으로 "앱이 넣은 것"과 "밖에서 넣은 것"을 구분한다.
  - 적용 대역 안에서 같은 router로 밖에서 추가된 route는 목록에 가져온다. 기존 대상과 겹치거나 같은 LAN 안이면 가져오지 않는다.
  - 다른 gateway를 거치는 route는 건드리지 않는다.
  - 목록에는 있는데 시스템에서 사라진 route는 다시 넣는다. 서비스가 새로 만들어지면 route가 통째로 비기 때문이다.
- **입력 규칙**: 한 목록 안의 항목끼리는 범위가 겹칠 수 없다. 설정창에서도 config.yaml에서도 오류로 처리한다.
- 쓰기에 실패하면 몇 번 다시 시도하고, 그래도 실패하면 macOS 알림과 설정창의 경고로 알린다.
- **감수하는 위험**: 외부 네트워크가 우연히 적용 대역과 같은 사설 대역을 쓰면 그곳에서도 적용된다.

### 5.6 UI
- 메뉴바 아이콘은 연결됨 / 끊김 / 연결·재연결 중 / 오류 네 가지만 구분한다.
- 설정창은 계정, 연결 관리, 로그 세 탭이다. 인증서 관련 항목은 없다.
- 기존 `~/.config/openfortivpn/config`가 있으면 첫 실행 때 값을 가져온다. 원본 파일은 수정하지 않는다.
- macOS 알림은 설정창이 닫혀 있어도 사용자가 알아야 하는 문제(예외 규칙 쓰기 실패)에만 쓴다.

## 6. 확인된 외부 제약
구현 전에 소스를 읽거나 이 Mac에서 직접 확인한 사실이다. 바꾸려면 다시 확인해야 한다.

**openfortivpn 1.24.1 / Apple pppd**
- `--persistent`는 이메일 OTP 경로에서 쓴 OTP를 다시 보낸다.
- password·OTP prompt에는 줄바꿈이 없다. 줄이 끝나기를 기다리면 prompt를 놓친다.
- stdout에는 openfortivpn 로그가, stderr에는 pppd 메시지와 route 오류가 나온다. **stderr가 곧 오류라는 뜻은 아니다.**
- 비밀값은 verbosity를 올렸을 때만 출력된다(cookie `-v`, OTP `-vv`, password `-vvv`).
- exit code는 원인과 무관하게 섞여 나오고, pppd의 끊김 메시지도 원인을 구분해 주지 않는다.
- 인증서 검증은 끌 수 없다. 거부할 때 stdout에 `trusted-cert = <digest>`를 출력하고, 인증 이후 단계에서도 다시 나올 수 있다. password는 TLS 검증을 통과한 뒤에야 전송된다.
- SIGTERM handler는 터널이 올라온 뒤에만 설치된다. 그 전에는 즉시 종료되며, spawn할 때 signal mask를 초기화해야 이렇게 동작한다.
- 비정상 종료하면 default route가 사라진다. 그래서 Helper에 route 복구가 있다.
- `https_proxy`/`all_proxy` 환경 변수를 읽는다.

**FortiGate**
- OTP 메일 제목은 `AuthCode: NNNNNN`이고 발신자는 일정하지 않다. 유효시간은 기본 60초다.
- 같은 IP에서 2회 실패하면 일정 시간 차단된다.
- FortiOS 7.6.3부터 SSL VPN tunnel mode가 제거됐다(§9).

**Gmail**
- IMAP `UID SEARCH UID n:*`는 n보다 작은 마지막 메일도 돌려준다. 그래서 클라이언트에서 한 번 더 거른다.
- `\All` 메일함에는 스팸함이 포함되지 않는다. OTP를 못 찾으면 스팸함을 한 번 확인해 원인으로 알려준다.
- IMAP은 Gmail 도착 후 노출까지 약 7초가 걸렸다.

**macOS**
- Personal Team Development 서명은 Release 빌드에도 `get-task-allow`를 넣는다. App Sandbox나 제한된 entitlement를 쓰면 7일짜리 provisioning profile이 붙으므로 **entitlement는 비워 둔다.**
- 서명 인증서가 유효하려면 로그인 Keychain에 Apple WWDR G3 중간 인증서가 있어야 했다.
- 재설치하면 실행 중인 옛 Helper는 디스크의 바이너리가 바뀌어 앱의 서명 확인을 통과하지 못한다. 그래서 Helper가 자기 실행 파일이 바뀐 것을 감지하면 유휴 상태일 때 스스로 종료한다. `make install`에 sudo가 필요 없는 이유다.
- 앱 target의 기본 격리가 MainActor라서, XPC 오류·무효화 callback을 `@Sendable`로 선언하지 않으면 런타임 격리 검사에서 크래시한다.
- `/sbin/route`와 `networksetup`은 실패해도 exit code 0으로 끝난다. 결과는 출력이나 변경 후의 routing table로 판단한다.
- networksetup이 다루는 저장된 additional routes에 macOS가 스스로 넣는 항목은 없다. macOS 자체 route(자기 IP `/32`, `169.254.0.0/16`)는 실행 상태에만 있고 networksetup에는 보이지 않는다.
- 화면 잠금은 `com.apple.screenIsLocked`/`Unlocked` 분산 알림이 기준이다. 잠금 해제 직후 세션 정보는 잠시 잠금 상태로 남아 있을 수 있다.
- main actor task 안에서 modal 창(`runModal`)을 열면 main queue가 멈춰 연결 처리, OTP 확인, 타이머가 모두 멈춘다. 파일 선택 창은 sheet로 연다.

## 7. 결정 이력과 교훈
같은 실수를 반복하지 않기 위해, 현재 구성에 이르게 한 결정과 그 근거를 남긴다.
- **2026-10-15 SMAppService 검증**: Personal Team 서명으로 등록, 승인, XPC 통신이 동작했다. 실패하면 쓰려던 레거시 LaunchDaemon 설치(최초 1회 관리자 암호)는 필요 없어졌다. 재설치 문제(§6 macOS)도 이때 발견했다.
- **2026-10-15 OTP 지연 측정 → Gmail API 추가**: IMAP으로 측정해 보니 prompt부터 코드 입력까지 약 10초가 걸렸다. 사용자가 즉시 입력을 원해서 Gmail API 방식을 추가하고 IMAP은 대체 경로로 남겼다.
- **2026-10-15 route 복구 검토**: `/sbin/route`가 실패해도 0으로 끝나서, 복구 후 routing table을 다시 확인하도록 바꿨다.
- **2026-10-16 OAuth client 내장**: client는 앱을 식별하는 값이라 사용자가 가져오게 하는 것은 불필요한 절차였다.
- **2026-10-19 로그 검토**: 37개 세션이 모두 앱의 사전 재연결(약 175분), 종료, 연결 해제로 끝났다. 서버가 세션을 끊는 경우는 아직 관찰되지 않았다. 로그의 "modem hanging up" 줄은 앱이 보낸 SIGTERM에 대한 pppd의 반응이지 서버의 종료가 아니다.
- **2026-10-23 스케줄**: 처음에는 종료 시각에 연결을 끊는 안을 검토했다. 사용자가 "끊지 말고 재연결만 하지 말라"고 정해서 자리 비움에 따라 재연결만 제한하는 지금의 규칙이 됐다.
- **2026-11-03 예외 규칙**
  - 재부팅과 VPN 재연결 뒤에도 유지되도록 networksetup additional routes를 택했다.
  - 같은 config.yaml을 여러 Mac에서 쓰기 위해 적용 대역을 두었다.
  - config.yaml에 오류가 있으면 연결도 막는 동작은 사용자가 확인한 결정이다.
- **2026-11-04 예외 대상 검증의 교훈**: TCP 연결만 보고 "직접 연결 가능"으로 판단했다가, 실제로는 출발지 IP를 검사하는 서버라 쓸 수 없었다. 대상이 직접 연결되는지는 실제 프로토콜로 확인한다. 직접 경로에서는 ping이 막혀 있어 ping 실패는 근거가 되지 않는다.
- **2026-11-05 정리**
  - "필요할지 몰라서" 넣었다가 쓰이지 않던 데이터 항목, 옵션, 테스트용 장치를 지웠다.
  - 원칙은 다음과 같다. 기능의 목적이 분명할 때 만든다. 테스트를 위해 제품 코드에 무언가를 두지 않는다. 세부 사항은 코드에 두고, 방향과 설계는 이 문서에 둔다.

## 8. 보안 요약과 감수하는 위험
- 비밀값은 Keychain에 저장하고, Helper에는 XPC로 메모리 안에서만 전달한다. argv, 설정 파일, 환경 변수, 로그에 남지 않는다.
- XPC는 양쪽 모두 Team ID와 signing identifier를 확인한다. Helper는 검증된 값만 받고, config.yaml은 읽지 않는다.
- **감수하는 위험** (개인 Mac, 사용자 결정)
  1. **서버 인증서 자동 신뢰**: 공용 Wi-Fi 등에서 능동적인 MITM 공격자가 가짜 인증서를 내밀면 자동으로 신뢰한다. 그러면 VPN 비밀번호와 실시간 OTP가 넘어갈 수 있다. OTP를 자동 제출하므로 사람이 알아챌 틈이 없다.
  2. **같은 사용자 권한의 코드가 root로 이어지는 경로**: Homebrew 바이너리 변조, Personal Team 키를 이용한 앱 사칭, `/Applications`의 앱 바꿔치기가 가능하다.
  3. **config.yaml 변조**: `vpn.host`를 바꾸면 1번과 결합해 자격증명이 다른 서버로 갈 수 있다. Keychain 항목이 `username@host` 단위라 host가 바뀌면 비밀번호를 다시 입력해야 하므로 부분적으로 완화된다.
- 강화 옵션(인증서 변경 시 확인, root 소유 바이너리 사본, 게이트웨이 고정과 관리자 인증)은 추후 과제다.

## 9. 리스크와 추후 과제
- FortiOS 7.6.3 이상으로 게이트웨이를 올리면 openfortivpn을 쓸 수 없다. 대안 후보는 openconnect `--protocol=fortinet`이다.
- 잠자기 중 PPP kernel extension panic 보고가 있다(openfortivpn #1278).
- health check가 프로세스와 interface로 한정돼 있어 서버 idle-timeout으로 죽은 연결은 늦게 감지될 수 있다.
- Personal Team 인증서는 1년마다 만료된다.
- openfortivpn을 업그레이드하면 출력 문자열이 바뀔 수 있다. 검증된 버전이 아니면 연결 관리 탭에 경고를 띄운다.
- 추후 과제: 지인 배포, 인증서 보안 강화, split DNS, 다중 프로필, 비정상 종료 뒤 터널 이어받기(adopt), root 소유 바이너리 사본, openconnect backend.

## 10. 수동 점검
큰 변경 뒤 실제 환경에서 확인할 시나리오다.
1. config.yaml이 없는 상태로 첫 실행: 기존 openfortivpn 설정을 가져오고, Helper 승인과 Gmail 연결 후 openfortivpn 1회 실행으로 연결된다.
2. `vpn.trusted_certs`를 비우고 연결: 인증서 거부 1회 뒤 즉시 재실행으로 연결되고 digest가 저장된다. 다음 연결부터는 거부 없이 1회에 연결된다.
3. pppd 또는 openfortivpn을 강제로 종료: route가 복구되고 재연결된다.
4. 세션 최대 시간을 짧게 바꿔 사전 재연결이 동작하는지 확인한다.
5. 짧은 잠자기, 긴 잠자기, 연결 중 잠자기, Wi-Fi 변경
6. 잘못된 VPN 비밀번호, 잘못된 Gmail 인증, OTP 메일 차단: 각각 정해진 대로 멈춘다.
7. config.yaml 직접 편집: 옵션 변경 반영, 잘못된 값 거부, 깨진 YAML에서 마지막 정상 설정 유지, symlink 유지
8. 앱 강제 종료와 정상 종료: 프로세스가 정리되고 route가 복원된다.
9. 보안: `ps`와 로그에 비밀값이 없고, 렌더링된 openfortivpn 설정 파일은 0600이며 종료 후 삭제된다. config.yaml과 state.json은 0600이다. `make verify`가 통과한다.
