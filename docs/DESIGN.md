# FortiVPNManager 설계

이 문서는 앱이 **왜 이렇게 만들어졌는지**를 기록한다. 배경, 목표, 핵심 결정과 그 이유, 구성 요소의 역할, 확인된 외부 제약, 결정 이력을 다룬다.
무엇을 하는지는 [SPEC.md](SPEC.md)가 기준이다. 값, 시간, 필드, 파일 구성처럼 바뀔 수 있는 세부 사항은 코드가 기준이다. 이 문서에 다시 적지 않는다.

## 1. 배경
- FortiGate SSL VPN 서버가 약 3시간마다 세션을 강제로 끊는다. 그때마다 `sudo openfortivpn`을 다시 실행하고 Gmail로 온 이메일 OTP를 손으로 입력해야 했다.
- 개인 Mac에서 쓰는 도구다. 지인 배포는 추후 과제이고, 유료 Apple Developer 계정은 없다.
- 회사 Windows PC에서도 같은 일을 하려 한다. Windows에서는 openconnect를 VPN client로 쓴다(§7.1, §7.2). openconnect는 나중에 macOS와 Linux에서도 쓸 수 있게 한다. Linux는 추후 과제다.

## 2. 목표와 범위
- **목표**
  - 메뉴바에 상주하면서 openfortivpn 실행, OTP 입력, 재연결을 사람 대신 한다.
  - 사용자가 신경 쓸 일을 최소로 줄인다. 인증서 확인 창, 수동 OTP 입력 창, 기술적인 설정 항목을 두지 않는다.
  - 다른 OS와 VPN client를 붙일 때 UI, OS 연동, VPN client 처리만 새로 만들면 되도록 공통 부분을 나눠 둔다. 지원하는 것은 macOS + openfortivpn과 Windows + openconnect다.
  - 한 기기의 config.yaml을 다른 OS로 그대로 옮겨 쓸 수 있게 한다. 비밀번호와 Gmail 인증만 그 기기에서 다시 넣는다.
- **범위 밖**
  - 앱이 비정상 종료된 뒤 실행 중인 터널을 다시 이어받는 기능(adopt)
  - split DNS, 다중 VPN 프로필
  - 보안 강화 옵션(§8)
  - 공증된 배포본

## 3. 핵심 결정

### 3.1 동작
| 결정 | 이유 |
|---|---|
| VPN client를 직접 만들지 않고 Homebrew openfortivpn을 감싼다 | 이미 이 게이트웨이에서 동작이 확인된 도구다. Homebrew 바이너리를 root로 실행하는 위험(§8)은 개인 Mac이라 감수한다 |
| root 작업은 SMAppService.daemon으로 등록한 Helper가 맡고, 앱과 XPC로 통신한다 | openfortivpn(pppd, routing table 변경)은 root가 필요하다. 매번 관리자 암호를 묻지 않으면서 root 권한이 필요한 범위만 좁게 분리하기 위해서다 |
| 무료 Apple ID(Personal Team)의 Apple Development 인증서로 서명한다 | launchd는 등록한 프로그램을 cdhash가 아니라 Team ID와 signing identifier로 식별한다. 그래서 재빌드해도 다시 승인받을 필요가 없다. 그 대가로 **번들 ID(`com.qoor.FortiVPNManager`, `…Helper`)는 최초 등록 후 바꾸지 않는다** |
| 연결할 때마다 openfortivpn 프로세스를 새로 띄운다 | `--persistent`는 이메일 OTP 경로에서 이미 쓴 OTP를 다시 보낸다 |
| 비밀번호와 OTP는 stdin pipe로만 전달한다 | argv, 설정 파일, 환경 변수, 로그 어디에도 비밀값을 남기지 않는다. PTY를 쓰지 않고 기본 verbosity로 고정하면 openfortivpn 원본 로그에도 비밀값이 나오지 않는다 |
| 종료 원인은 openfortivpn 출력이 아니라 Helper가 기록한 사실로 판단한다 | openfortivpn의 exit code와 pppd 메시지만으로는 원인을 구분할 수 없다(§6). 대신 어느 단계까지 진행했는지, 누가 종료시켰는지, 터널이 얼마나 유지됐는지를 본다 |
| 서버 인증서는 무조건 자동 신뢰한다 | 사용자가 인증서를 신경 쓰지 않기를 원했다. openfortivpn이 인증서를 거부하면서 출력하는 SHA-256 digest를 config.yaml `vpn.trusted_certs`에 저장하고, 이후 연결부터는 이 값을 넘긴다. 그래서 평소에는 openfortivpn을 한 번만 실행한다. 거부는 비밀번호를 보내기 전에 일어나므로 계정 잠금과는 무관하다 |
| OTP는 Gmail API(OAuth)로 읽고, 실패하면 IMAP IDLE(앱 비밀번호)로 전환한다 | IMAP은 메일이 Gmail에 도착한 뒤 IMAP에서 보이기까지 약 7초가 늦었다. Gmail API는 그보다 빠르지만 즉시는 아니다(2026-11 로그에서 도착 후 감지까지 중앙값 약 4.6초, 3~12초). API 인증이 실패하고 앱 비밀번호가 있으면 같은 시도 안에서 IMAP으로 넘어간다. IMAP은 설정이 더 쉽고 일반적이라 계속 둔다 |
| OAuth Desktop client는 빌드할 때 앱에 넣는다 | client는 사용자가 아니라 앱을 식별하는 값이므로 사용자가 직접 가져올 이유가 없다. Desktop client의 secret은 비밀로 취급되지 않고 PKCE로 보호된다. 다른 client를 쓰고 싶으면 계정 pane의 고급 항목에서 가져와 덮어쓸 수 있다 |
| 사용한 OTP 메일은 휴지통으로 옮긴다 | 다음 시도에서 이전 코드를 잘못 고르지 않게 하고, 받은편지함에 OTP 메일이 쌓이지 않게 한다 |
| 수동 OTP 입력 창은 만들지 않는다 | 자동화가 목적이다. OTP를 읽지 못하면 실패로 처리한다 |
| 인증 실패는 보수적으로 센다 | FortiGate는 같은 IP에서 2회 실패하면 일정 시간 차단한다. 그래서 단순한 통신 끊김일 가능성이 있어도 로그인 단계에서 끝난 시도는 인증 실패로 센다. 두 번째 실패에서는 멈추고 사용자의 [연결]을 기다린다. 네트워크 오류는 세지 않고 backoff한다 |
| 세션 만료 전에 미리 재연결한다(사전 재연결) | 작업 도중 갑자기 끊기는 대신, 정해 둔 시각에 기존 세션을 먼저 끊고 새로 연결한다(break-before-make) |
| 잠자기 중에는 터널을 유지하고, 깨어나면 상태를 보고 판단한다 | 짧은 잠자기마다 재연결해서 OTP 메일이 오는 일을 막는다 |
| health check는 프로세스와 ppp interface만 본다 | Apple pppd에는 LCP echo가 없고 openfortivpn에는 network timeout이 없다. 그래서 별도 ping 대상을 두지 않는 대신 연결 단계마다 watchdog을 둔다 |
| 설정은 config.yaml, 비밀번호는 Keychain, 기기별 일시 상태는 state.json에 둔다 | config.yaml은 사람이 편집할 수 있고 dotfiles로 옮길 수 있다. 로그인 시 열기 여부는 macOS(SMAppService)가 소유하는 상태라서 YAML에 두지 않는다 |
| 이전 빌드와 다음 빌드는 서로가 남긴 저장소를 읽을 수 있어야 한다 | 사용자 결정이다(Rust 빌드부터). config.yaml, state.json, Keychain 항목, Helper의 실행 기록이 해당한다. 앱 ↔ Helper 메시지는 같은 빌드끼리만 오가므로 해당하지 않는다(§4.2) |
| config.yaml은 엄격하게 읽는다 | 사람이 직접 편집하는 파일이다. YAML 1.1의 8진수·60진수 해석, anchor/alias처럼 의도와 다르게 읽힐 수 있는 문법은 거부한다. 파일에 오류가 있으면 마지막 정상 설정을 유지하고 연결을 막는다 |
| 앱이 config.yaml을 다시 쓰면 직접 단 주석과 이 버전이 모르는 키는 사라진다 | 사용자가 수락한 한계다. 앱이 파일을 쓰는 경우는 설정창 저장, 새 인증서 digest 기록, 예외 규칙 가져오기뿐이다 |
| 로그는 마스킹하지 않은 원본을 보여준다 | 기본 verbosity에서는 비밀값이 출력되지 않음을 openfortivpn 소스에서 확인했다. 문제를 추적할 때는 원본이 가장 유용하다 |
| Windows에서는 openconnect(`--protocol=fortinet`)를 감싼다 | openfortivpn은 Windows를 공식 지원하지 않는다. 사용자 PC의 FortiClient 7.4.3에는 CLI가 없고, 회사 설치본이라 올릴 수 없다. openconnect로 같은 게이트웨이에 비밀번호와 이메일 OTP로 로그인하는 것을 실제 PC에서 확인했다(§6) |
| openconnect의 인증서 고정 값은 config.yaml의 새 키(`vpn.trusted_cert_pins`)에 둔다 | 값의 형식이 openfortivpn과 달라(`pin-sha256:`) 같은 목록에 섞을 수 없다. 섞으면 이전 빌드가 형식 오류로 경고한다. 새 키는 이전 빌드가 모르는 키로 보고 경고만 하므로 빌드끼리의 호환을 지킨다 |
| openconnect에는 인증서 고정 값을 항상 하나 이상 넘긴다 | 고정 값 없이 실행하면 pipe에 아무것도 내보내지 않고 멈췄다. 맞지 않는 값을 넘기면 실제 값을 출력하고 곧바로 끝난다. 그래서 openfortivpn과 같은 "거부 → 저장 → 즉시 재시도" 흐름이 되고, 인증서 질문에 답하는 장치가 필요 없다 |
| openconnect 처리는 OS와 무관하게 두고, OS마다 다른 것만 나눈다 | openconnect는 나중에 macOS와 Linux에서도 쓴다. 실행 인자, prompt, 출력 문구, 인증서 값 형식, 세션 cookie와 logout은 OS와 무관하다. OS마다 다른 것은 실행 파일 위치, 넘길 환경 변수, 터널 어댑터를 만들었다는 줄, route 스크립트, 정상 종료 요청 수단이다. 인증서 목록은 OS가 아니라 VPN client를 따른다 |
| openconnect는 로그인과 터널을 두 번의 실행으로 나눈다 | Helper가 logout하려면 세션 cookie가 필요하다(아래). 연결하면서 cookie를 출력하는 옵션은 Windows에서 openconnect가 정상 종료할 때까지 출력 버퍼에 남아, 강제로 끝내면 잃는다. 로그인만 하는 실행은 cookie를 출력하고 정상 종료하므로 곧바로 받는다. 받은 cookie를 stdin으로 넘기면 둘째 실행이 터널을 연다. OS와 무관한 흐름이라 다른 OS에서도 그대로 쓴다 |
| Helper가 세션 cookie를 가진 채 openconnect를 강제로 끝내면 게이트웨이에 logout 요청을 보낸다 | 강제로 끝내면 openconnect가 logout하지 못해 세션이 idle timeout(50분)까지 서버에 남는다. 따로 보낸 logout 요청으로 세션이 끝나는 것을 확인했다(§6). 정상 종료 요청으로 끝나면 openconnect가 스스로 logout하므로 보내지 않는다. 그래서 signal이 있는 OS에서는 유예를 넘겨 강제로 끝낼 때만 보내고, Windows에서는 늘 보낸다 |
| logout 요청은 OS의 curl로 보낸다 | Helper에 TLS와 인증서 고정 확인 코드를 따로 넣지 않아도 된다. Windows 10 이후에는 관리자만 바꿀 수 있는 System32에 있고, macOS·Linux에도 기본으로 있다. 인증서 고정 값이 맞을 때만 서버와 통신하는 것을 확인했다. cookie는 stdin으로 넘기고, curl 설정 파일과 proxy 설정은 읽지 않게 한다 |
| openconnect 연결 성공은 터널 어댑터를 만든 시점으로 본다 | `Configured as` 줄은 어댑터가 생기기 약 1초 전에 나온다. 그 뒤 route가 다 들어가기까지 몇 초가 더 걸리지만 기다리지 않는다. macOS도 route를 확인하지 않고, 그동안 생기는 일은 새로 여는 연결이 잠깐 VPN을 타지 않는 정도다. route 스크립트가 실패하는 일이 실제로 보이면 그때 확인을 더한다 |
| Windows에서는 openconnect를 늘 강제로 끝내고, 그것이 띄운 route 스크립트도 함께 끝낸다 | Windows에는 signal이 없다. openconnect가 logout하는 정상 종료 수단은 console 이벤트뿐인데, 다른 프로세스의 console에 붙어 이벤트를 보내는 시험 실행 파일을 회사 PC의 보안 프로그램(AhnLab MDS)이 악성으로 막았다(2026-11-12). 그래서 늘 강제로 끝내고 Helper가 logout한다. 터널 직후에 끝내면 route 스크립트가 아직 돌고 있어, 함께 끝내지 않으면 정리한 게이트웨이 host route를 스크립트가 다시 넣을 수 있다 |
| Windows는 터널이 올라온 뒤 요청 없이 끝난 openconnect를 모두 강제 종료로 본다 | 사용자 결정이다(2026-11-11). signal이 없어서 밖에서 끝낸 것을 구분할 수 없다. 로그인은 이미 성공했으므로 FortiGate 차단과 무관하다. 접속 직후 끊기는 일이 반복돼도 곧바로 재연결은 한 번뿐이고, 그 뒤 backoff하다 시도 상한에서 멈춘다 |
| Windows 예외 규칙은 주 interface의 persistent route로 둔다 | OS가 저장해 두고 다시 넣으므로 macOS의 additional routes와 성질이 같다. persistent 목록에는 VPN 스크립트가 넣는 실행 중 route가 없어서 "앱이 넣은 것"과 "밖에서 넣은 것"의 구분이 그대로 성립한다. VPN이 받은 대역보다 좁은 예외 route가 먼저 선택되는 것을 실제 PC에서 확인했다 |
| 언어 | 코드, 주석, 앱과 Helper가 남기는 로그는 영어로 쓴다. UI 한국어는 String Catalog로 번역한다. UI에서 "헬퍼"가 아니라 "Helper"라고 쓴다. Rust가 통신하거나 파일을 읽다 실패했을 때의 상세 설명은 라이브러리 원문(영어)을 그대로 보인다(§7.2) |

### 3.2 구조와 기술
| 결정 | 이유 |
|---|---|
| 공통 로직은 Rust로, macOS의 UI·OS 연동·OS 연동 Helper는 Swift로 만든다 | 다른 OS와 VPN client를 붙일 때 공통 부분을 그대로 쓰기 위해서다. **macOS UI는 앞으로도 Swift로 만든다.** OS 기본 UI가 가장 단순하고 반응이 빠르며, OS에 의존하는 영역을 다른 언어로 옮기면 복잡도만 는다 |
| Swift ↔ Rust는 프로세스 안의 이름 없는 socketpair 위로 4바이트 big-endian 길이와 UTF-8 JSON을 주고받는다 | UniFFI는 설정 부담이 크고 C binding용 unsafe가 늘어난다. 별도 프로세스 IPC는 필요 없다. Rust의 시작 함수(앱은 런타임, Helper는 Helper entry)가 socketpair를 만들어 한쪽 fd를 Swift에 넘긴다. Swift는 FileHandle과 Data만 쓰고 pointer 코드가 없다. 읽는 쪽은 하나이고, 쓰기는 background serial queue에서 하며, main actor 전환은 한 곳에서 한다 |
| 메시지 타입은 Rust가 기준이다 | Swift 쪽 Codable 정의는 Rust가 만든 예시 JSON(fixture)으로 맞는지 테스트한다. gRPC, MessagePack, protobuf는 쓰지 않는다 |
| UI 상태는 도메인 구역 단위로 보낸다 | 화면 모양의 전체 상태는 비효율적이고 공통 부분을 화면 변경에 묶는다. 연결, 스케줄, 준비 상태·Helper, Gmail·Google 계정, 예외 규칙, 설정 구역 중 바뀐 것만 통째로 보내고, 로그는 추가분만 보낸다. 판단(버튼 상태, "다음 연결부터 적용" 등)은 공통 부분이 하고, 문장은 모두 UI가 만들어 번역한다 |
| Rust에 unsafe 블록을 두지 않는다 | 사용자 결정이다. 외부로 내보내는 시작 함수의 이름 표시만 허용한다 |
| SIGPIPE: Rust가 socketpair 양쪽에 SIGPIPE 금지를 켜고, Helper 프로세스는 SIGPIPE 전체를 무시한다 | main이 Swift라 Rust의 SIGPIPE 무시가 적용되지 않는다. macOS에서 pipe 하나에만 SIGPIPE를 막는 설정은 unsafe 없이 할 수 없어서, Helper 전체의 무시를 watchdog의 전제 조건으로 둔다. VPN client는 모든 signal을 기본값으로 되돌린 상태로 시작한다 |
| 내부 오류는 곧바로 중단한다 | 억지 예외 처리 대신 panic은 abort하고 crash report를 남긴다. Release 빌드에도 줄 번호를 남긴다 |
| config.yaml은 saphyr-parser의 이벤트 API로 읽고 엄격 규칙은 직접 확인한다 | pure safe Rust다. 구조를 재귀 대신 반복으로 만들어 큰 스택이 필요 없다 |
| TLS는 rustls에 ring provider 하나와 platform verifier를 쓴다 | OS의 신뢰 저장소를 따른다. aws-lc-rs는 넣지 않는다. HTTPS는 OS의 proxy 설정을 따른다 |
| Windows의 UI와 OS 연동은 C# WinUI 3로 만든다 | 사용자 결정이다(2026-11-12, 처음에는 WPF). macOS에서 Swift를 고른 이유와 같다(OS 기본 UI). WinUI 3는 Windows에서만 빌드되지만 빌드와 시험을 Windows PC에서 하므로 문제가 없다 |
| Windows OS 연동은 화면 없는 C# 라이브러리로 두고 WinUI 앱이 가져다 쓴다 | 런타임 연결, Helper 통로, OS 기능을 UI 없이 시험할 수 있다. Helper 통로의 규칙(같은 빌드로 확인된 연결, 다시 시도, 응답 시간 제한)은 macOS의 Swift와 같다. interface는 Windows가 보여 주는 어댑터 이름으로 부르고, 경로·주소·네트워크 변경 이벤트가 모두 같은 이름을 쓴다. 디스플레이 꺼짐, 앱 활성화, 알림은 창이 있어야 받거나 보일 수 있어 UI와 함께 둔다 |
| Windows 앱도 런타임을 같은 프로세스에서 시작하고 같은 JSON 메시지를 쓴다 | Rust 시작 함수가 익명 pipe 두 개를 만들어 handle 값만 돌려준다. C#은 이를 stream으로 감싸 읽고 쓰므로 둘 사이에 pointer가 오가지 않는다. C# 쪽 메시지 정의는 Swift처럼 Rust가 만든 fixture로 검사한다 |
| Windows Helper는 Rust만으로 된 Windows 서비스(LocalSystem)다 | 서비스 진입, 연결 통로, 프로세스 처리를 모두 Rust 라이브러리로 할 수 있어 macOS의 Swift 껍데기 같은 OS 쪽 코드가 필요 없다. 서비스 시작 callback은 인자를 읽지 않으므로 unsafe가 필요 없다 |
| Windows 앱 ↔ Helper는 named pipe 위의 같은 메시지다 | XPC 자리다. 통로는 설치할 때 정한 사용자와 시스템만 열 수 있게 접근 권한을 둔다. 앱은 통로가 시스템 계정 소유인지 확인한 뒤에 비밀번호를 보낸다. 같은 이름의 통로를 다른 프로그램이 먼저 만들어 가로채는 일을 막기 위해서다. 연결한 프로그램이 설치본인지는 unsafe 없이 확인할 방법이 없어 확인하지 않는다(§8) |
| Windows의 route 읽기·쓰기는 PowerShell route cmdlet을 고정 argv로 실행하고 JSON으로 받는다 | macOS의 networksetup처럼 시스템 명령을 쓴다. `route print`와 `route.exe`는 출력이 번역돼 해석할 수 없고, route API를 직접 부르면 unsafe가 필요하다. 바꿀 값은 stdin의 JSON으로 넘기고, 명령 문장은 고정한다 |
| Windows 앱 ↔ Helper 통로의 메시지는 앱 안의 Rust ↔ Swift와 같은 틀(4바이트 길이와 UTF-8 JSON)이고, 요청마다 번호를 붙인다 | XPC의 호출과 응답 자리다. 응답은 요청 번호로 짝을 찾고, 이벤트와 출력은 생긴 순서대로 같은 통로로 간다. 빌드 확인과 유휴 시 종료 요청은 다른 빌드 사이에서 오가므로 모양을 바꾸지 않는다. 앱은 빌드 확인에 자기 빌드를 함께 보내고, Helper는 같은 빌드로 확인된 연결의 연결·OTP·예외 route 요청만 처리한다. Helper가 받는 한 메시지는 1 MiB까지고, 예외 route 요청은 연결마다 한 번에 하나만 받는다(앱도 하나씩 보낸다). 앱이 읽지 않아 보낼 메시지가 256개 넘게 쌓인 연결은 앱이 끝난 것처럼 다룬다. 같은 사용자의 다른 프로그램도 연결할 수 있어서, 큰 길이를 보내거나 요청만 쌓아 Helper의 메모리를 다 쓰게 하지 못하게 한다. 그 통로는 앱이 다음에 쓰거나 끝날 때 닫힌다. 막혀 있는 통로를 밖에서 끊는 안전한 방법이 없기 때문이다 |
| Windows Helper는 유휴 시 종료 요청에 끝나지 않는다고 답한다 | Windows는 스스로 끝난 서비스를 다음 요청 때 다시 띄우지 않는다. 재설치가 서비스를 멈추고 바꾼 뒤 다시 시작하므로, 빌드가 다르면 설치가 끝나지 않은 것이다. 그때 앱은 버전 불일치를 보인다 |
| 통로를 열 사용자는 Helper 서비스의 실행 인자로 받는다 | 설치 스크립트가 정해 서비스 설정에 넣는다. 서비스 설정은 관리자만 바꿀 수 있다. 사용자 이름은 바뀔 수 있어서 SID로 넣는다. 통로는 시스템 계정이 소유하고, 그 사용자에게는 통로를 새로 만드는 권한을 주지 않는다. 그 권한이 있으면 같은 이름의 통로를 하나 더 열어 앱의 연결을 가로챌 수 있고, 소유자 확인으로도 구분되지 않는다 |
| Windows Helper의 실행 기록은 시스템 계정의 프로필 폴더(`C:\Windows\System32\config\systemprofile\AppData\Local\FortiVPNManager`)에 둔다 | 시스템과 관리자만 접근할 수 있는 곳이라 권한을 물려받는다. 폴더 권한을 직접 다루면 unsafe가 필요하다. ProgramData는 일반 사용자도 그 안에 폴더를 먼저 만들 수 있어서 쓰지 않는다 |
| Windows Helper의 watchdog은 프로세스를 sysinfo로 확인한다 | 실행 파일 경로, 시작 시각, 사용자를 unsafe 없이 읽는다. 시작 시각은 초 단위다. pid, 시작 시각, 경로가 모두 같고 Helper와 같은 사용자(시스템 계정)인 프로세스만 기록한 그 프로세스로 본다 |
| Windows Helper의 로그는 Windows 이벤트 로그에 쓴다 | macOS 통합 로그의 자리다. 서비스가 시작할 때 시스템 권한으로 로그 출처를 스스로 등록하므로 설치 스크립트가 할 일이 없다 |
| Windows Helper는 예외 route 요청의 대상을 판단 코어의 규칙으로 확인한다 | 앱이 계획할 때 쓰는 규칙과 같아야 하기 때문이다. macOS에서는 Swift가 같은 규칙을 따로 갖고 있다. 판단 코어는 바깥과 통신하지 않으므로 Helper에 넣어도 하는 일이 늘지 않는다 |

## 4. 구성과 역할

### 4.1 앱과 Helper
- **앱** (사용자 권한, 메뉴바 상주)
  - 모든 판단을 맡는다. 연결 state machine, OTP 처리, 재연결 스케줄, config.yaml·Keychain·state.json, 서버 인증서 digest, 예외 규칙 계산, UI, 로그 보관이 여기에 속한다.
  - Swift의 UI와 OS 연동이 Rust 런타임을 같은 프로세스에서 시작한다. 판단과 공통 I/O는 런타임이, 화면과 macOS 기능(Keychain, XPC, 시스템 이벤트, 알림, 로그인 항목)은 Swift가 맡는다.
- **Helper** (root LaunchDaemon, 요청이 있을 때 launchd가 실행)
  - 실행하고 감시하는 일만 한다. openfortivpn 실행, stdin으로 비밀번호·OTP 주입, prompt 감지, 단계별 watchdog, 프로세스 종료, 비정상 종료 뒤 routing table 복구, 남은 프로세스 정리, 네트워크 서비스의 additional routes 읽기·쓰기가 여기에 속한다.
  - Swift의 OS 연동 Helper(XPC, launchd, 서명 확인, route 복구, networksetup)가 Rust Helper entry를 같은 프로세스에서 시작한다. Helper entry는 앱 요청을 판단하고, VPN client에 관한 일은 backend adapter를 쓰는 watchdog에 맡긴다.
  - 상태를 저장하지 않고 config.yaml도 읽지 않는다. 앱이 보낸 값 중 검증을 통과한 것만 쓴다. 환경 변수는 PATH만 넘긴다(openfortivpn이 proxy 환경 변수를 읽기 때문, §6).
- **둘을 나눈 이유**: root로 실행되는 코드를 최소로 줄이기 위해서다. 앱은 root 권한 없이 자유롭게 재시작하거나 재설치할 수 있다.
- **Windows**: 나눔과 역할이 같다. 앱은 사용자 권한의 C# 앱(UI와 OS 연동)이 Rust 런타임을 같은 프로세스에서 시작한다. Helper는 부팅 때 시작하는 Rust Windows 서비스(LocalSystem)이고, macOS Helper의 Swift 부분이 하던 일(연결 확인, route 정리, 예외 route 쓰기)도 직접 한다.

### 4.2 앱과 Helper 사이의 규칙
- XPC 연결은 양쪽이 서로의 code signing requirement(Team ID + signing identifier)를 확인한다. 비밀번호는 Helper의 버전을 확인한 뒤에만 보낸다.
- 앱과 Helper는 같은 빌드여야 한다(BuildID 비교). 다르면 연결하지 않고 "Helper 버전 불일치"로 표시한다. 그래서 둘 사이의 메시지 형식은 구버전 호환을 고려하지 않는다.
  - 예외: 버전 확인과 옛 Helper에 대한 종료 요청은 다른 빌드 사이에서 오가므로 형식을 바꾸지 않는다.
- XPC로 오가는 내용은 공통 계약이 정한 JSON이다. 앱의 OS 연동은 내용을 읽지 않고 런타임에 그대로 넘기며, 직접 읽는 것은 버전뿐이다.
- Helper는 앱의 연결이 끊기면(종료, 강제 종료, 크래시) 터널을 정상 종료한다. 그래서 세 경우가 모두 같은 경로를 탄다.
- Helper가 보내는 이벤트와 로그 출력은 하나의 순서 있는 채널로 전달된다. 연결 해제나 종료가 끝났는지는 요청에 대한 응답이 아니라 Helper가 보내는 종료 이벤트로 판단한다.
- Windows에는 서명 확인이 없다(유료 인증서가 없다). 그 대신 Helper는 통로의 접근 권한으로 상대 사용자를 제한하고, 앱은 통로의 소유자가 시스템 계정인지 확인한다. 같은 빌드인지 확인하는 규칙과 메시지 형식은 macOS와 같다.

### 4.3 구성 요소
구성 요소마다 디렉토리를 하나 둔다. 구성 요소 안에서 OS에 의존하는 부분만 `macos/` 같은 하위 디렉토리로 나눈다. Rust crate 이름에는 `fvm-` 접두사를 붙인다(Rust의 기본 라이브러리 `core`와 겹치지 않게).

```
components/
  contract/                     공통 계약: backend 인터페이스, 앱 ↔ Helper 메시지, 앱 ↔ 런타임 메시지, 메시지 읽기·쓰기
                                (macos/: Swift 쪽 정의와 XPC 연결 정의, windows/: C# 쪽 정의)
  core/                         판단 코어: 연결 state machine, config.yaml, 예외 규칙 계산, IMAP·Gmail API·OAuth·OTP 메일 형식. I/O 없음
  runtime/                      런타임: 조율과 타이머, OTP 세션, Google 계정, 설정·상태·로그 파일, 예외 규칙 동기화, UI 메시지
                                (src/macos/, macos/, src/windows.rs: 각 OS 앱이 런타임을 시작하는 부분)
  backend-adapter/openfortivpn/ backend adapter: 실행 인자와 설정, 출력 해석, 종료 유예 시간, 검증된 버전, 기존 설정 가져오기
  backend-adapter/openconnect/  backend adapter: 두 번의 실행 인자, 출력 해석, 세션 cookie, logout 요청, 검증된 버전
                                (OS마다 다른 것: 실행 파일 위치, 환경 변수, 터널 어댑터 줄. 지금은 Windows만)
  watchdog/                     watchdog: VPN client 확인, 실행 준비와 설정 파일, 프로세스 실행, stdin, 출력, 단계별 시간 제한,
                                종료, 종료 사실, 실행 기록, 이전 Helper가 남긴 프로세스 정리
                                (unix/, windows/: 프로세스 실행·확인·종료, 실행 기록 폴더 권한)
  helper-entry/                 Helper entry: 두 OS가 함께 쓰는 앱 요청 처리, 실행의 주인 판단, Helper 수명. OS 전용 API 없음
                                (macos/: Helper의 시작 함수. backend adapter와 실행 기록 폴더를 골라 만든 watchdog을
                                 Helper entry에 넘긴다. windows/: Helper 서비스 실행 파일)
  ui/macos/                     UI: 메뉴바와 설정창, 모든 문장과 번역
  ui/windows/                   UI: 알림 영역 아이콘과 설정창, 모든 문장과 번역
  os-integration/macos/         OS 연동: port 구현, 앱 진입점, 런타임 메시지 전달
  os-integration/windows/       OS 연동: port 구현, 앱 진입점, 런타임 메시지 전달, Helper 통로
  os-integration-helper/macos/  OS 연동 Helper: XPC, launchd, 서명 확인, route 복구, networksetup
packaging/macos/                Xcode 프로젝트 정의, 빌드 설정, LaunchDaemon 정보, 빌드·설치 스크립트
packaging/windows/              C# solution, 빌드·설치 스크립트
```

- **의존 방향**: 판단 코어 → 공통 계약. 런타임 → 판단 코어, backend adapter, 공통 계약. backend adapter → 공통 계약. watchdog → 공통 계약. Helper entry → watchdog, 공통 계약. Helper의 시작 함수 → Helper entry, watchdog, backend adapter, 공통 계약. Helper 안에서 backend adapter를 쓰는 것은 watchdog뿐이고, 시작 함수는 어느 것을 쓸지 고르기만 한다. Windows Helper 서비스는 여기에 더해 판단 코어(예외 route 값 확인)를 쓴다.
- Windows의 Helper 서비스는 macOS의 OS 연동 Helper 자리를 따로 두지 않는다. 서비스 진입, 통로, route 정리를 Rust로 할 수 있어서다.
- 판단 코어와 watchdog은 VPN client 이름을 모른다. 런타임이 backend adapter를 쓰는 곳은 기존 설정 가져오기, 인증서 값 형식 확인, 계정 값 확인 세 가지뿐이다.
- **앱 안의 흐름**: UI는 명령을 보내고, OS 연동은 시스템 이벤트와 Helper 메시지를 보낸다. 런타임은 바뀐 도메인 구역, 로그 추가분, port 요청(Keychain, Helper 호출, 네트워크 조회, 브라우저 열기, 로그인 항목)과 port 동작(시스템 로그, 알림, 활동 유지, 종료 승인)을 돌려보낸다. 전체 그림은 SPEC §1.1에 있다.
- **다른 OS를 붙일 때**: UI, OS 연동(port 구현), 서비스 진입점, watchdog의 OS 부분만 새로 만든다. Linux와 Windows의 서비스는 Helper entry와 watchdog을 그대로 쓰고, backend adapter와 실행 기록 폴더를 골라 시작하는 부분만 맡는다.

### 4.4 빌드와 설치
- Rust는 2024 edition이고 Apple Silicon 전용으로 빌드한다. XcodeGen의 pre-build script가 cargo로 정적 라이브러리를 만들고, module map이 링크할 라이브러리를 지정한다. libproc crate가 bindgen을 쓰므로 빌드에 libclang(Xcode에 포함)이 필요하다.
- `make test`는 Rust 테스트 전체와 Swift 테스트 묶음(공통 계약, Helper, 앱의 UI·OS 연동)을 돌린다.
- `make install`은 앱 종료, 교체, 재실행을 분리된 background 작업으로 한다. 앱을 끄면 VPN이 끊기고, 설치를 실행한 세션이 VPN을 타고 있으면 그 세션과 함께 설치가 중간에 죽을 수 있기 때문이다. 설치 전에 VPN이 연결되어 있었는데 새 빌드로 터널이 돌아오지 않으면 이전 앱을 되돌려 놓는다.
- **Windows**: x64용으로 빌드한다. 빌드와 테스트는 사용자의 Windows PC에서 MSVC toolchain으로 한다. 런타임은 Windows 빌드 때만 DLL로 만든다. 빌드 설정에 DLL을 넣으면 macOS에서도 함께 생겨, 앱이 정적 라이브러리 대신 그 파일에 연결될 수 있기 때문이다. 빌드는 사용자 자신의 계정으로 빌드 스크립트(`packaging/windows/build.ps1`)를 실행해 release 빌드를 만든다. Rust는 사용자마다 설치되므로 관리자 계정에는 없을 수 있다. 설치는 관리자 권한으로 한 번 실행하는 스크립트다(`packaging/windows/install.ps1`). 그 빌드를 Program Files에 넣고, 서비스를 등록하고, 통로를 열 사용자를 정한다. 앱과 Helper는 같은 빌드 번호를 갖고, 형식은 macOS 빌드와 같다. 서비스는 부팅 때 시작하고, 비정상 종료하거나 시작에 실패할 때마다 다시 시작한다(5초, 5초, 그 뒤 1분마다). Helper 없이는 VPN을 쓸 수 없고, 통로 이름을 다른 프로그램이 먼저 차지한 경우처럼 원인이 사라지면 저절로 돌아오기 때문이다. 이 사용자는 화면에 로그인한 사용자가 기본이고 직접 지정할 수도 있다. 일반 사용자가 설치하면 스크립트가 관리자 계정으로 돌아서, 실행한 계정을 고르면 정작 사용자의 앱이 연결하지 못하기 때문이다. 앱이 쓸 이벤트 로그 출처도 여기서 등록한다(등록에만 관리자 권한이 필요하다). Windows가 꺼질 때 종료 준비 알림을 받은 Helper를 기다려 줄 시간(30초)도 서비스 설정에 넣는다. sc.exe에는 이 설정이 없어 서비스 API를 직접 부른다. 재설치는 서비스 정지 → 교체 → 시작이다. 새 파일은 옆 폴더에 먼저 두고, Helper 프로세스까지 끝난 뒤 앱 폴더를 이름 바꾸기 한 번으로 통째로 바꾼다. 앱처럼 폴더 안의 프로그램이 실행 중이면 이름 바꾸기가 실패하고, 그때는 아무것도 바꾸지 않은 채 Helper를 다시 시작하고 끝난다. PowerShell의 폴더 옮기기는 이름 바꾸기가 실패하면 파일을 하나씩 옮겨, 쓰이는 파일만 남기고 Helper 실행 파일은 옮겨 버린다(2026-11-13 노트북에서 확인). 회사 보안 프로그램이 새 실행 파일을 몇 분 붙잡아 Windows가 서비스 시작에 주는 30초를 넘기므로, 시작이 실패하면 10분까지 다시 시도한다. OpenConnect가 설치돼 있지 않으면 알리고 설치는 계속한다. 앱의 준비 상태에도 나오기 때문이다.

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
  2. 터널이 올라온 뒤 앱도 Helper도 멈추지 않은 openfortivpn이 signal로 끝났다: 누군가 강제로 끝낸 것이다. 서버나 계정 탓이 아니므로 유지 시간과 관계없이 실패로 세지 않고 곧바로 재연결한다. Windows의 openconnect는 어떻게 끝났든 여기에 넣는다(§3.1).
  3. 안정적으로 유지되던 터널이 우리 요청 없이 끝났다: 서버가 세션을 끝낸 것으로 보고 잠시 뒤 재연결한다.
  4. 게이트웨이에 닿기 전에 끝났다: 네트워크 오류로 보고 backoff한다.
  5. 로그인 단계에서 끝났다: 인증 실패로 센다. 단, 시도 중 네트워크가 바뀌었거나 깨어난 직후라면 네트워크 오류로 본다.
  6. OTP 미수신, OTP 거부, 인증 후 터널 실패, 너무 빨리 끊김: 인증 실패로 센다.
- Helper와의 연결이 끊기면 강제 종료로 다룬다. 이 연결은 Mac 안에만 있어서 네트워크 때문에 끊기는 일이 없고, Helper 프로세스가 끝날 때만 끊긴다.
- 곧바로 재연결은 안정 세션이 생길 때까지 한 번만 한다. 그 사이의 강제 종료는 네트워크 오류처럼 backoff한다. Helper가 연결할 때마다 죽으면 VPN 로그인과 OTP 메일이 끝없이 반복되기 때문이다.
- 곧바로 재연결도 다른 재시도와 같은 조건(잠자기, 네트워크, 스케줄, 시도 상한)을 거친다.
- 인증 실패가 1회면 잠금이 풀린 뒤 한 번 더 시도하고, 2회면 멈춘다. 일정 시간 안의 시도 횟수에도 상한을 둔다.
- Gmail 인증 자체가 실패하면 openfortivpn을 실행하지 않고 멈춘다. 잘못된 Gmail 설정으로 VPN 로그인 시도를 낭비하지 않기 위해서다.
- 멈춘 상태는 사용자가 [연결] 또는 [지금 재연결]을 눌러야 풀린다. 실패 횟수는 state.json에 저장되므로 앱을 다시 실행해도 유지된다.

### 5.3 잠자기와 깨어남
- 연결 중이거나 재시도를 기다리는 중에 잠들면 시도를 중단한다. 깨어나 네트워크가 돌아오면 새로 시도한다.
- 연결된 상태에서 깨어나면 다음 경우에만 재연결한다. 사전 재연결 시각이 지났을 때, 프로세스나 ppp interface가 사라졌을 때, 게이트웨이까지의 물리 경로(underlay)가 바뀌었을 때, 사용자가 켜 둔 "잠자기 후 재연결" 조건을 만족할 때다. 그 밖에는 유지한다.
- 타이머는 벽시계 기준이다. 잠자기나 시계 변경 뒤에도 정해 둔 시각이 지났으면 곧바로 동작한다.

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
- **Windows 방식**: 주 interface(VPN 어댑터 제외)의 persistent route를 그 gateway로 쓴다. 성질과 규칙은 macOS와 같고, 앱이 쓴 route는 interface GUID 단위로 기억한다. 밖에서 바뀐 것을 알려 주는 알림이 없어, OS 연동이 주소가 바뀔 때와 5분마다 네트워크 설정이 바뀌었다고 알리고 런타임이 다시 맞춘다.
  - 주 interface는 하드웨어 Ethernet·Wi-Fi 어댑터 가운데 default route의 metric이 가장 낮은 것이다. VPN 터널 어댑터는 이 종류가 아니어서 빠진다. FortiClient나 Hyper-V의 가상 어댑터는 Ethernet으로 보고되므로 하드웨어 여부로 뺀다. Hyper-V 외부 스위치처럼 가상 어댑터가 실제 연결을 맡은 PC에서는 주 interface가 없다.
  - 앱에는 interface GUID를 서비스 ID로, interface 이름을 서비스 이름으로 보낸다. route 목록은 그 interface의 persistent route에서 default route를 뺀 것이다. 고정 IP로 설정한 interface는 default route도 persistent 목록에 두기 때문이다.
  - 쓸 때는 지금 목록과 비교해 빠진 것을 지우고 새 것을 넣는다. persistent 목록과 지금 쓰는 목록 둘 다에 한다.
  - 앱이 읽은 interface, router, 주소, route가 그대로인지는 쓰기와 같은 PowerShell 실행 안에서 확인한다. 따로 확인하면 PowerShell이 다시 뜨는 1~3초 사이에 DHCP 갱신 등으로 router가 바뀌어도 옛 router로 쓰게 된다.
  - route 하나를 바꾸는 데 노트북에서 약 0.05초가 걸린다(2026-11-13). 쓰기의 제한 시간은 기본 20초에 바꿀 route마다 0.2초를 더해, 런타임이 한 번에 바꾸는 최대치(지울 것 100개와 넣을 것 100개)도 60초 안에 들게 한다. 앱은 route 읽기·쓰기의 답을 90초 기다린다. 앱이 먼저 포기하면 Helper가 아직 처리 중인 그 요청 때문에 다음 route 요청을 거절하기 때문이다.
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
macOS의 Apple Human Interface Guidelines를 따른다. 다른 OS의 UI는 플랫폼과 구현 언어가 달라도 사용자에게 같은 구성으로 보인다(사용자 결정, 2026-11-11). 같은 창과 메뉴, 같은 pane을 같은 순서와 같은 pane 고르는 방식(창 왼쪽의 사이드바와 설정 검색)으로 두고, pane 안의 항목도 같은 묶음과 순서로 둔다. 같은 구성이란 화면 모양보다 설정이 어느 pane과 묶음에 속하는지를 같게 한다는 뜻이다(사용자 설명, 2026-11-12). 컨트롤은 그 OS의 기본 컨트롤로 그린다. 다른 점은 OS에 없는 것뿐이다. Windows는 메뉴바 대신 알림 영역 아이콘에서 같은 메뉴를 열고, 그 OS에 해당하지 않는 항목(기존 openfortivpn 설정 가져오기, Helper 승인)은 빠진다. 다른 OS의 설정창이 따를 pane 구성, 저장 방식, 설정 검색 규칙, 문구 원칙은 [SETTINGS-UI.md](SETTINGS-UI.md)에 모았다.
- 메뉴바 아이콘은 연결됨 / 끊김 / 연결·재연결 중 / 오류 네 가지만 구분한다.
- **앱 아이콘**: 원형 화살표가 방패를 감싼 모양(VPN과 자동 재연결)이고, 색은 파랑 하나다. Fortinet과 공식 FortiClient로 오해받지 않도록 빨간색은 쓰지 않는다.
  - macOS는 기본 앱처럼 흰 배경에 그림에만 색을 넣는다. Icon Composer 문서로 두어 Dark, Clear, Tinted 모습은 macOS가 그린다.
  - Windows는 같은 그림을 배경판 없이 쓴다. 32px 이하는 선을 굵게 한 그림이다.
- **메뉴바 메뉴**: 상태와 연결 동작을 둔다.
  - 연결된 동안 터널 IP와 연결 시각을 보인다.
  - [연결] [연결 해제] [지금 재연결]은 늘 보이고, 지금 쓸 수 없으면 흐리게 한다.
  - 설정이 필요하면 빠진 항목마다 동작을 하나씩 두고, 그 항목을 고치는 pane을 연다. 한 줄로 묶어 "필요: …"와 [설정 열기]를 두었을 때는 Helper 문제인지 설정 문제인지 알 수 없었다.
  - 연결에 실패하면 안내와 함께 도움이 되는 곳(계정, 고급, 로그 pane)을 여는 버튼을 둔다.
- **설정창**: macOS 시스템 설정 형태다(사용자 결정, 2026-11-12). 창 왼쪽 사이드바 위의 설정 검색과 pane 목록으로 pane을 고른다.
  - pane은 일반(연결 상태와 버튼, 로그인 시 열기, 앱 시작 시 자동 연결, openfortivpn 설정 가져오기), 계정, 연결 관리(스케줄, 재연결), 예외 규칙, 고급(Helper, VPN client, 설정 파일), 로그 순이다.
  - 창은 시스템 설정과 같은 740×625pt 고정 크기이고 전체 화면이 없다. 처음에는 화면 가운데에, 그 뒤로는 마지막 위치에 열린다. 그 위치가 화면을 벗어나면 화면 안으로 옮긴다. 창 제목은 pane 이름이고, 다시 열면 마지막 pane이 열린다.
  - 도구막대에는 뒤로/앞으로와 그 pane의 동작(계정의 되돌리기·저장, 예외 규칙의 변경 취소·적용, 로그의 찾기·지우기·모두 복사·Finder에서 보기)을 두고, 같은 동작을 파일·편집·보기 메뉴에도 둔다.
  - 설정 검색은 pane과 설정의 이름을 앱 언어와 영어로 찾는다. 결과를 고르면 그 pane으로 가서 접힌 묶음을 펼치고, 그 행으로 스크롤해 잠시 강조한다. 한글은 macOS 27 시스템 설정과 같은 규칙으로 맞춘다. 앞 글자는 음절 그대로, 입력기가 조합 중인 마지막 글자만 자모 단위로 맞추고 초성 검색은 하지 않는다.
  - 로그는 설정이 아니지만, 사용자가 별도 창 대신 pane으로 정했다.
  - 인증서 관련 항목은 없다.
  - 설명문은 한두 문장으로 두고, 자세한 설명은 마우스를 올리면 나오는 help tag에 둔다.
  - 창이 열려 있는 동안 앱은 일반 앱이 되어 Dock 아이콘과 표준 메뉴(앱, 파일, 편집, 보기, 윈도우)를 가진다.
- UI는 판단하지 않는다. 받은 도메인 구역을 그리고, 사용자의 조작을 명령으로 보낸다. 로그 pane의 [모두 복사]는 UI가 화면에 가진 줄로 만든다.
- 기존 `~/.config/openfortivpn/config`가 있으면 첫 실행 때 값을 가져온다. 원본 파일은 수정하지 않는다.
- macOS 알림은 설정창이 닫혀 있어도 사용자가 알아야 하는 문제(예외 규칙 쓰기 실패)에만 쓴다. 알림을 누르면 예외 규칙 pane이 열린다.

## 6. 확인된 외부 제약
구현 전에 소스를 읽거나 이 Mac에서 직접 확인한 사실이다. 바꾸려면 다시 확인해야 한다.

**openfortivpn 1.24.1 / Apple pppd**
- `--persistent`는 이메일 OTP 경로에서 쓴 OTP를 다시 보낸다.
- password·OTP prompt에는 줄바꿈이 없다. 줄이 끝나기를 기다리면 prompt를 놓친다.
- stdout에는 openfortivpn 로그가, stderr에는 pppd 메시지와 route 오류가 나온다. **stderr가 곧 오류라는 뜻은 아니다.**
- 비밀값은 verbosity를 올렸을 때만 출력된다(cookie `-v`, OTP `-vv`, password `-vvv`).
- exit code는 원인과 무관하게 섞여 나오고, pppd의 끊김 메시지도 원인을 구분해 주지 않는다.
- 인증서 검증은 끌 수 없다. 거부할 때 stdout에 `trusted-cert = <digest>`를 출력하고, 인증 이후 단계에서도 다시 나올 수 있다. password는 TLS 검증을 통과한 뒤에야 전송된다.
- SIGTERM handler는 터널이 올라온 뒤에만 설치된다. 그 전에는 즉시 종료되며, spawn할 때 signal mask를 초기화해야 이렇게 동작한다. 터널이 올라온 뒤 SIGTERM을 받으면 스스로 정상 종료(exit code 0)하므로 서버 종료와 구별되지 않는다. SIGKILL로 끝내야 signal 종료로 남는다. pppd만 끝내도 openfortivpn은 signal 없이 스스로 끝난다.
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
- main actor에 격리된 Swift 코드에서 XPC 오류·무효화 callback을 `@Sendable`로 선언하지 않으면 런타임 격리 검사에서 크래시한다.
- `/sbin/route`와 `networksetup`은 실패해도 exit code 0으로 끝난다. 결과는 출력이나 변경 후의 routing table로 판단한다.
- networksetup이 다루는 저장된 additional routes에 macOS가 스스로 넣는 항목은 없다. macOS 자체 route(자기 IP `/32`, `169.254.0.0/16`)는 실행 상태에만 있고 networksetup에는 보이지 않는다.
- 화면 잠금은 `com.apple.screenIsLocked`/`Unlocked` 분산 알림이 기준이다. 잠금 해제 직후 세션 정보는 잠시 잠금 상태로 남아 있을 수 있다.
- main actor task 안에서 modal 창(`runModal`)을 열면 main queue가 멈춰 연결 처리, OTP 확인, 타이머가 모두 멈춘다. 파일 선택 창은 sheet로 연다.
- Rust의 `main`이 아닌 프로세스(Swift 앱에 링크된 Rust 라이브러리)에서는 SIGPIPE가 무시되지 않는다. 닫힌 상대에게 쓰면 프로세스가 죽는다.
- `xcodebuild`는 String Catalog에 새 문장을 넣지 않는다. Xcode에서 빌드하거나, 빌드가 남긴 `.stringsdata`로 `xcstringstool sync`를 실행한다.

**openconnect 9.21 / Windows 11** (사용자 PC, 2026-11-06·11-10 시험. 회사 FortiClient가 설치된 상태)
- `--protocol=fortinet`으로 같은 게이트웨이에 비밀번호와 이메일 OTP로 로그인했다.
  - `Password:`와 `Code:` prompt는 stderr에 줄바꿈 없이 나온다. 비밀번호는 TLS 검증 뒤에 묻는다.
  - 진행 줄(`Connected to HTTPS on`, `Got Legacy IP address`, `Configured as`)은 stdout에 나온다.
- realm은 접속 URL의 경로로 넘긴다(소스 확인).
- **인증서**
  - 고정 값의 형식이 openfortivpn과 다르다(`pin-sha256:`, SPKI base64). openfortivpn의 digest는 맞지 않는다.
  - 고정 값 없이 실행하면 "signer not found" 뒤 pipe에 아무것도 내보내지 않고 기다렸다. Ctrl+C 이벤트에도 끝나지 않았다.
  - 맞지 않는 값을 넘기면 실제 값을 stderr에 출력하고 바로 exit 1로 끝난다. 묻지 않는다.
- 비밀번호 미리 넣기 옵션은 인증서 질문이 그 줄을 읽고, 비대화 옵션은 OTP 질문까지 막는다(소스 확인). 로그인이 거부되면 끝나지 않고 다시 묻는다.
- **터널이 올라오는 순서**
  - OTP를 넣은 뒤 약 1초에 `Configured as`가 나온다.
  - 약 1초 뒤 Wintun 어댑터가 생긴다(stderr `Using Wintun device '…', index N`).
  - 그 뒤 vpnc-script-win.js가 route를 넣는다. 2026-11-10에는 route 29개가 `Configured as` 뒤 6~8초에 다 들어갔다. 2026-11-06의 수동 실행과 2026-11-12 시험에서는 openconnect의 10초 스크립트 제한을 넘겨 "Script did not complete within 10 seconds"와 "Failed to spawn script" 줄이 나왔지만 터널은 유지됐다. 소스상 openconnect는 이 결과를 무시하고 스크립트를 끝내지도 않는다.
  - 스크립트의 출력은 openconnect의 pipe로 오지 않는다.
- 게이트웨이는 DNS를 주지 않는다(macOS openfortivpn도 같다).
- **끊김** (2026-11-11 시험: 어댑터 끄기, 게이트웨이만 방화벽으로 막기)
  - 게이트웨이는 끊긴 세션으로 다시 붙는 것을 허용하지 않는다. openconnect가 다시 붙으려 하면 서버가 거부하고(HTTP 302 또는 403), openconnect는 logout한 뒤 exit 2로 끝난다. exit 2는 이 경우에만 나온다(소스 확인).
  - 다시 붙으려는 시도는 기본값으로 300초 동안 10초, 20초, 30초… 간격으로 한다. 대기 시간을 다 써도 마지막 간격만큼 쉰 뒤 한 번 더 시도한다. 대기를 0으로 하면 첫 재시도가 실패하는 즉시 끝난다(소스 확인).
  - 어댑터가 꺼지면 곧바로 알아챈다. 선은 살아 있고 게이트웨이와의 통신만 끊기면 약 20초 뒤 dead peer로 알아챈다.
  - 다시 시도하는 동안에도 터널 어댑터는 Up으로 남는다. 통신이 끊긴 것은 openconnect가 끝날 때에야 알 수 있다.
  - 스스로 끝날 때는 게이트웨이 host route도 지운다.
  - 유선이 끊기자 Wi-Fi가 곧바로 붙어 openconnect가 그 경로로 서버에 닿았지만 역시 거부됐다.
- 데이터는 DTLS(UDP)로 보낸다. openfortivpn은 TLS(TCP)만 쓴다.
- 같은 계정의 동시 세션이 허용된다. Windows에서 세 번 로그인하는 동안 Mac 세션이 유지됐다.
- OTP를 67초 뒤에 넣었더니 거부되고 Code를 다시 물었다. 56초 뒤에는 받아들여졌다. 오래 기다리면 서버가 연결을 닫지만 openconnect가 새 연결로 다시 보낸다.
- **종료**
  - 터널이 올라온 뒤 다른 프로세스가 openconnect의 console에 붙어 Ctrl+C 이벤트를 보내면 logout하고 route를 되돌린 뒤 2초 안에 exit 0으로 끝난다.
  - Ctrl+C를 무시하는 설정은 자식 프로세스에 물려진다. SSH로 접속한 세션은 명령을 이 설정으로 실행해서, 그 아래에서 띄운 프로세스는 Ctrl+C를 받지 못한다. Windows가 시작한 서비스에는 이 설정이 없다. 시스템 계정 서비스 안에서 띄운 프로세스에 같은 서비스 안에서 보낸 Ctrl+C가 전달됐다(2026-11-11).
  - 회사 PC의 AhnLab MDS Agent가 다른 프로세스의 console에 붙어 Ctrl+C를 보내던 시험 실행 파일을 "악성 파일 진단 후 차단"으로 막았다(탐지명 없음, 2026-11-12). 처음 보는 실행 파일은 몇 분씩 분석한 뒤에 실행된다.
  - 소스상 Ctrl+C와 console 닫힘·logoff·shutdown 이벤트는 logout하는 취소이고, Ctrl+Break는 logout 없이 세션을 남기고 끝나는 detach다. Ctrl+Break만 process group 하나로 보낼 수 있으므로, 보낸 쪽을 피하려고 Ctrl+Break를 쓰면 logout이 되지 않는다.
  - 강제 종료하면 터널 어댑터와 주소는 사라지지만 게이트웨이로 가는 host route가 남는다. 어댑터는 숨은 상태로 남았다가 다음 openconnect 실행이 지운다. 그래서 어댑터 이름이 매번 같지 않다.
  - 강제 종료 직후 다시 로그인할 수 있었다.
  - 프로세스 트리째 강제로 끝내면 openconnect가 띄운 route 스크립트도 함께 끝난다.
- **세션 cookie와 logout** (2026-11-12 시험)
  - 로그인만 하는 실행은 stdout에 `COOKIE='SVPNCOOKIE=…'`와 HOST, CONNECT_URL, FINGERPRINT, RESOLVE 줄을 내고 exit 0으로 끝난다. 그 cookie를 stdin으로 받은 둘째 실행이 터널을 열었다.
  - 연결하면서 cookie를 출력하는 옵션은 출력을 비우지 않아, stdout이 pipe일 때 정상 종료할 때까지 나오지 않는다(소스 확인).
  - 강제 종료 뒤 2·30·90초에도 서버는 그 cookie를 세션으로 다뤘다. 세션 정보 요청에 접근 거부(HTTP 403)가 왔고, 끝난 세션이나 가짜 cookie에는 로그인 페이지로 가라는 응답(HTTP 302)이 온다. logout 요청 뒤에는 끝난 세션과 같은 응답이 왔다.
  - logout 요청의 응답은 가짜 cookie에도 같다(HTTP 200, 같은 길이). 그래서 응답으로 성공 여부를 알 수 없다.
  - Windows의 curl은 `-k`를 줘도 공개 키 고정 값이 틀리면 통신하지 않는다.
- Wintun은 openconnect 설치본에 들어 있고, 쓸 때마다 "experimental" 경고를 낸다. 날짜와 Windows 오류 문장 같은 일부 출력은 시스템 언어로 번역되고 UTF-8이 아니다.

**Windows route**
- `route -p add`로 넣은 route는 persistent 목록에 들어가고 곧바로 쓰인다. VPN이 받은 /24보다 좁은 /32 예외 route가 먼저 선택됐고, VPN 정상 종료와 강제 종료 뒤에도 남았다.
- PowerShell로 넣어도 기본으로 지금 쓰는 목록과 persistent 목록 둘 다에 들어간다. 다시 시작한 뒤에도 남아 선택됐다. 어댑터를 끄면 잠시 빠졌다가 다시 켜면 저절로 돌아온다(2026-11-11).
- `route.exe`와 `route print`의 출력은 시스템 언어로 번역된다.

## 7. 결정 이력과 교훈
같은 실수를 반복하지 않기 위해, 현재 구성에 이르게 한 결정과 그 근거를 남긴다.

### 7.1 기능
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
- **2026-11-09 강제 종료 뒤 곧바로 재연결**
  - kill 테스트에서 재연결이 정해진 대기 시간만큼 늦었다. openfortivpn을 SIGKILL로 끝내면 서버 종료로 분류돼 잠시 기다렸고, 터널이 올라온 직후였다면 "너무 빨리 끊김"으로 인증 실패에 세어져 더 오래 기다렸다. signal 종료는 서버나 계정과 무관하므로 강제 종료로 따로 분류하고 곧바로 재연결한다. 터널이 올라오기 전의 signal 종료는 FortiGate의 IP 잠금 때문에 단계 규칙을 그대로 둔다. pppd만 끝내면 openfortivpn이 signal 없이 스스로 끝나서 서버 종료와 구분할 수 없어 그대로 둔다.
  - Helper를 `kill -9`로 끝내면 네트워크 오류로 다뤄 backoff만큼 기다렸다. Helper와의 연결은 Mac 안에만 있어서 끊겼다면 언제나 Helper 프로세스가 끝난 것이지 네트워크 문제가 아니다. 그래서 강제 종료로 다룬다.
  - Helper는 내부 오류에서 곧바로 중단한다(fail fast). 연결할 때마다 중단되면 곧바로 재연결이 VPN 로그인과 OTP 메일을 끝없이 반복하므로, 안정 세션이 생길 때까지 곧바로 재연결은 한 번만 한다.
- **2026-11-10 구조 단순화 검토**
  - crate 구성, 메시지 방식, 상태 소유는 그대로 두었다. 같은 판단·상수·상태의 사본을 지우고, 앱이 쓰지 않던 Helper 기능(빈 시도 ID로 실행 중인 것을 아무거나 멈춤)을 뺐다.
  - 교훈: 비밀번호를 읽는 사이에 정해진 중단 요청이 연결 요청보다 먼저 Helper에 가서, 이미 포기한 시도가 실행될 수 있었다. 준비를 기다린 뒤 보내는 요청은 보내기 직전에 판단 코어가 아직 원하는지 확인한다.
  - 재연결 시간의 대부분은 OTP 메일 감지 대기다. 앱 코드의 대기는 1초 미만이라, 더 줄이려면 Gmail 쪽 감지 방식을 먼저 측정해야 한다.
- **2026-11-10 HIG 기준 UI 정리**
  - Windows 지원과 openconnect backend를 붙이기 전에 했다. 정보 구성을 먼저 확정하면 Windows UI는 그것을 한 번만 따라 만들면 되기 때문이다. Rust 쪽은 바뀌지 않았다. UI에 보내는 정보가 화면 단위가 아니라 주제 단위라서 pane을 나눠도 macOS UI만 고치면 됐다.
  - 실제 macOS 컨트롤로 만든 목업을 사용자가 직접 써 보고 정했다. 그 과정에서 바뀐 것: 로그는 별도 창 대신 pane, 일반 pane에도 상태 표시, 설정 필요 안내는 항목별 동작.
  - openconnect가 들어올 자리: backend 선택은 고급 pane의 VPN client 묶음에 둔다. 설정 가져오기는 계정 pane에 그대로 두고 backend에 따라 달라진다.
- **2026-11-10 Windows + openconnect 계획과 PC 시험**
  - 시험 스크립트로 사용자 PC에서 확인한 것을 바탕으로 정했다(§6). 정상 종료, 강제 종료 뒤 정리와 재로그인, 예외 route, 동시 세션이 이때 확인됐다.
  - 바뀐 계획: 인증서 질문에 답하는 대신 고정 값을 항상 넘긴다. 연결 성공은 `Configured as`가 아니라 터널 어댑터를 만든 시점으로 본다.
  - 사용자 결정: UI는 C# WPF, openconnect 고정 값은 새 키, driver는 openconnect에 들어 있는 Wintun(TAP은 문제가 보이면 검토), macOS는 openfortivpn을 그대로 쓰고 openconnect는 Windows에서만 검증, 정상 종료는 Rust unsafe 한 곳. 빌드 환경은 사용자가 나중에 정한다. 네트워크 끊김과 재부팅 확인은 구현한 뒤에 한다.
  - 사용자 결정(2026-11-11): 연결한 프로그램이 설치본인지는 확인하지 않고 통로를 설치한 사용자로 제한한다(§8). Windows는 터널 뒤 요청 없는 종료를 모두 강제 종료로 본다. Mac에서 Windows 코드를 컴파일하는 확인은 보류한다.
- **2026-11-11 Windows 끊김과 재부팅 시험**
  - 구현 전에 사용자 PC에서 했다(§6). 끊긴 세션은 서버가 매번 거부해서 openconnect가 스스로 다시 붙은 적이 없다.
  - openconnect의 재접속 대기를 0초로 정했다. 기다려서 얻는 것이 없고, 기다리는 동안 터널 어댑터가 남아 앱은 연결됨으로 보이기 때문이다. 기본값 300초면 최대 5분 동안 죽은 터널을 연결됨으로 보인다.
  - 앱이 따로 통신 상태를 확인하지 않는다. 통신만 끊기는 경우도 openconnect가 약 20초 안에 알아채고 끝난다.
  - 예외 route는 재부팅과 어댑터 끄고 켜기 뒤에도 남았다.
- **2026-11-11 UI 구성 통일**: 사용자 결정이다. 처음에는 Windows 설정창을 Windows 설정 앱처럼 왼쪽 목록으로 pane을 고르게 했다. 사용자가 플랫폼과 구현 언어가 달라도 같은 구성으로 보이기를 원해서, 창·메뉴·pane과 pane 안 항목의 배치를 macOS와 같게 하고 컨트롤만 그 OS의 것을 쓴다.
- **2026-11-12 시스템 설정 형태의 설정창**: 사용자 결정이다.
  - 실제 macOS 컨트롤로 만든 목업을 사용자가 macOS 27 시스템 설정과 비교해 보며 정했고, 앱은 그 형상을 따른다. 목업에 없던 것은 런타임이 보내는 상태(설정 필요 항목, 오류, 가져오기 보고 등)뿐이다.
  - 바뀐 것: pane 버튼 대신 사이드바와 설정 검색, 고정 크기 창, 저장·적용·로그 동작을 아래 막대에서 도구막대와 메뉴로 이동, openfortivpn 설정 가져오기를 계정에서 일반으로 이동, 펼친 항목을 각자의 행으로, 정보만 전하는 확인창 없음.
  - 2026-11-11의 구성 통일은 설정이 속한 pane과 묶음을 같게 한다는 뜻이라고 사용자가 설명했다. pane 고르는 방식도 macOS를 따르므로 다른 OS의 설정창도 사이드바로 고른다.
  - 예외 규칙은 런타임이 첫 문제 하나만 설명하므로, 그 행 옆에만 짧은 설명을 두고 다른 문제 행은 표시만 한다(사용자 결정: 이번에는 UI만 바꾼다). 도움말 버튼은 사용자용 도움말 문서가 없어서 두지 않았다.
- **2026-11-12 Windows UI를 WinUI 3로 변경**: 사용자 결정이다. WPF를 고를 때의 이점이던 Mac에서의 컴파일은, 빌드와 시험을 Windows PC에서 하게 되면서 쓸 일이 없어졌다.
- **2026-11-12 Windows 종료 방식 변경**
  - Ctrl+C를 보내는 시험 실행 파일을 회사 PC의 보안 프로그램이 악성으로 막았다(§6). 실제 Helper도 같은 PC에서 돌기 때문에 이 방식을 버렸다.
  - 강제 종료만 하면 세션이 서버에 남고, 게이트웨이 host route가 남고, 터널 직후라면 route 스크립트가 정리 뒤에 route를 다시 넣을 수 있다. 앞의 둘은 Helper가 logout과 route 정리를, 마지막은 프로세스 트리째 끝내는 것으로 막는다.
  - 사용자가 Helper의 logout 방식(A안)을 골랐다. 실제 로그인으로 강제 종료 뒤 세션이 남고 logout 요청으로 끝나는 것을 확인한 뒤 정했다. Rust unsafe 예외가 없어졌다.
  - 사용자 지시: openconnect는 Windows 전용이 아니다. 나중에 macOS와 Linux에서도 쓰므로 openconnect 처리는 OS와 무관하게 두고 OS마다 다른 것만 나눈다.
- **2026-11-12 공통 구조 정리**
  - Windows와 openconnect를 붙이기 전에 따로 했다(사용자 결정). 동작은 바뀌지 않는다.
  - Helper entry에서 앱 요청 처리, 실행 기록, 남은 프로세스 정리를 macOS의 시작 부분과 나눴다. 다른 OS의 Helper가 이 부분을 그대로 쓰게 하려는 것이다. OS마다 다른 것(프로세스 확인과 종료, 실행 기록 폴더 권한)은 backend adapter처럼 OS별 시작 부분이 넘겨준다. 그래서 Helper entry에는 OS 전용 코드가 없다. 어떤 backend adapter를 쓸지도 OS별 시작 부분이 정한다.
  - watchdog은 OS마다 다른 프로세스 처리만 따로 둔다. 실행 명세의 설정 파일은 없어도 되게 했다. 모든 것을 인자로 받는 VPN client도 있기 때문이다. VPN client 값 검사 가운데 client끼리 같은 것은 한곳에 모았다.
- **2026-11-12 Windows Helper 서비스**
  - macOS에서 Swift가 하던 일(앱 연결 받기, 버전 확인, route 정리, 예외 route)을 Rust 서비스가 직접 한다. 공통 Helper entry는 그대로 쓴다.
  - 공통 Helper entry의 로그와 설명에서 SIGTERM을 "종료 요청"으로 바꿨다. Windows의 서비스 정지도 같은 요청으로 들어오기 때문이다. 실행 파일 교체와 유휴 시 종료는 macOS만 보내므로 그 문구(launchd)는 그대로 둔다.
  - 프로세스 번호는 부호 없는 수로 다룬다. Windows의 pid가 그렇고, macOS의 pid도 양수만 쓴다. pidfile에 적히는 값은 같다.
  - Windows가 꺼질 때 서비스에 오는 종료 알림은 몇 초만 기다려 준다(노트북은 5초). Helper가 openconnect를 끝낸 뒤 보내는 logout이 잘릴 수 있어서, 다른 서비스와 네트워크가 내려가기 전에 오는 종료 준비(preshutdown) 알림을 받아 실행을 끝낸다.
- **2026-11-13 앱 아이콘**: 사용자 결정이다. 메뉴바 아이콘과 같은 방패 계열로 하고, 시안 캔버스에서 모양·색·Windows 배경판 여부를 골랐다.
  - 처음 시안의 화려한 배경색, 두 색을 섞은 그라데이션, 반쪽만 밝은 방패는 macOS 27 기본 앱 옆에서 이질감이 커서 버렸다. 기본 앱은 선명한 시스템 색 하나에 밝기만 살짝 바꾸고, 판의 70% 안팎을 채우는 크고 굵은 그림에 그림자를 거의 두지 않는다.
- **2026-11-14 VPN client는 watchdog만 다룸**: 사용자 결정이다. watchdog이 VPN client의 실행과 수명을 맡는데, Helper entry가 backend adapter를 직접 부르는 것은 잘못이라고 사용자가 정했다. Windows 작업을 올리기 전에 따로 했고, 앱이 보는 동작은 바뀌지 않는다.
  - watchdog으로 옮긴 것: client 확인(상태 응답에 쓰는 경로, 버전, 검증 여부, 문제), 실행 준비(실행 명세와 설정 파일), OTP 형식 확인, 메시지에 쓰는 client 이름, 실행 기록과 그에 담는 게이트웨이 조회, 이전 Helper가 남긴 client 프로세스 정리. 2026-11-12에 OS별 시작 부분이 넘겨주던 OS 처리(프로세스 확인과 종료, 실행 기록 폴더 권한)도 watchdog의 OS 부분이 됐다.
  - Helper entry에 남은 것: 앱 연결, 누가 무엇을 할 수 있는지(사용자와 시도의 주인), Helper 자신의 수명(유휴 종료, 교체, 종료 요청), 이전 Helper가 남긴 것의 정리와 route 복구가 끝날 때까지 앱 연결 막기, 실행이 요청한 route 복구를 OS 쪽에 넘기기, watchdog이 알려 준 것으로 만드는 응답, 로그. 시작 함수는 backend adapter와 실행 기록 폴더를 골라 watchdog을 만들어 넘긴다.
  - watchdog이 한 일에 관한 Helper 로그 줄은 watchdog이 만들어 Helper entry를 거쳐 내보낸다. 로그 문구, 실행 기록 폴더의 파일 이름과 형식은 그대로라서 새 Helper가 이전 Helper의 기록을 읽어 정리할 수 있다.

### 7.2 Rust 전환 (2026-11-05 ~ 11-09)
- **방향**
  - Windows의 FortiClient는 7.4.6부터 CLI(`FortiVPN.exe --cli`)가 있지만, 사용자 PC의 7.4.3은 회사 설치본이라 올릴 수 없었다. 그래서 Windows backend는 openconnect로 정하고 실제 PC에서 로그인, OTP, 트래픽을 확인했다(§6).
  - macOS는 openfortivpn을 기본으로 두고 나중에 openconnect를 고를 수 있게 한다. Windows는 openconnect로 고정하고, Linux는 둘 다 지원한다.
  - 이번 작업은 구조만 준비하는 사전 리팩토링이다. openconnect backend는 만들지 않았다. 공통 부분(판단, 공통 I/O, VPN client 처리)을 Rust로 옮기고 OS 앱에는 UI와 OS 연동만 남겼다.
- **구조 결정**
  - 처음에는 UniFFI를 검토했다. 사용자가 C binding용 unsafe가 늘어나는 것에 반대했고 설정 부담도 컸다. 그래서 socketpair 위의 JSON 메시지로 바꿨다(§3.2).
  - 공통 crate 5개(공통 계약, 판단 코어, 런타임, backend adapter, watchdog)와 macOS 전용 Helper entry 하나로 나눴다. backend adapter와 watchdog은 합치지 않았다. VPN client마다 바뀌는 부분과 프로세스 처리를 나눠야 유지보수가 쉽고, watchdog은 공통 계약에만 의존한다.
  - 디렉토리는 구성 요소 기준으로 나눴다. 언어별(`crates/`)이나 플랫폼별(최상위 `macos/`) 배치는 사용자가 거절했다.
  - UI 상태는 화면 모양의 전체 상태 대신 도메인 구역으로 보낸다(§3.2).
  - Linux UI는 나중에 Rust + QML(cxx-qt)로 만든다. Windows UI는 그 뒤 C# WPF로 정했다(§7.1).
- **진행 방식과 교훈**
  - 코드 이야기 없이 기능만 적은 명세(SPEC.md)를 먼저 썼다. 처음 쓴 초안은 코드를 산문으로 옮긴 것이라 다시 썼다.
  - 옛 코드는 바꿀 수 없는 외부 사실(XPC 메시지 모양, 시간 값과 횟수, openfortivpn 출력 문구와 실행 조건)을 확인할 때만 봤다. 옛 구조와 흐름은 따르지 않고 설계에서 출발했다. 옛 Swift 테스트는 Rust로 옮겨 같은 결과가 나오는지 확인했다.
  - 단계마다 자동 테스트, 리뷰, 실제 설치를 거쳤다. 단계는 기반 검증(Rust 연결, socketpair, 통신, YAML 선택), 파일 이동, Helper 쪽, 앱 쪽(한 번에 전환), 문서 순서였다.
  - 강제 종료 시험은 SIGKILL로 한다. openfortivpn은 SIGTERM을 받으면 스스로 정상 종료해서 서버 종료 경로를 탄다(§6).
  - 설치는 VPN을 끊으므로, 터널이 돌아오지 않으면 이전 앱으로 되돌리는 단계를 넣었다(§4.4).
- **저장소 호환**: 전환할 때 config.yaml, state.json(실패 기록 포함), Keychain 항목, 로그 파일(위치, 줄 형식, 회전)을 그대로 이어받았다. 앱 ↔ Helper의 XPC 메시지도 Swift 빌드와 같은 형식으로 시작했고, 바뀐 것은 Helper 상태의 실행 파일 문제(아래 3)뿐이었다.
- **Swift 빌드 호환 제거 (2026-11-09)**: 앱이 배포된 적이 없어서, Swift 빌드와 호환하려고만 둔 장치를 지웠다. state.json의 버전 키, Swift 표기를 흉내 낸 Helper 로그 문구, Swift 인코더에 맞춘 시각 표기가 여기에 해당한다. 그 대신 Rust 빌드끼리의 호환은 지킨다(§3.1).
- **승인된 동작 변경**: 사용자가 보는 동작은 바꾸지 않는 것이 원칙이었고, 아래만 승인을 받아 바꿨다.
  1. config.yaml에 raw NEL·LS·PS가 있으면 오류로 처리한다. YAML 1.1과 1.2가 이 문자를 줄바꿈으로 볼지 다르게 정한다.
  2. YAML parser를 바꾸면서 일부 오류의 줄·열 위치와 드문 문법의 처리가 조금 달라졌다(block scalar와 일부 문법 오류의 위치, flow 중첩 255단계 제한, `key:<TAB>value`는 문법 오류, 따옴표 값이나 키의 tag 거부). 값을 다르게 읽는 경우는 없다.
  3. openfortivpn 실행 파일 문제는 종류와 경로로 넘기고, UI가 문장을 만들어 번역한다. 전에는 Helper의 영어 문장을 그대로 보였다.
  4. Rust가 통신하거나 파일을 읽다 실패했을 때의 상세 설명(계정 pane의 Gmail 연결 테스트, Google 계정 연결, 기존 설정 가져오기)은 라이브러리 원문(영어)을 그대로 보인다. 전에는 macOS가 만든 한국어 설명이었다. 사용자가 실제로 보고 필요하면 따로 개선을 요청한다.
  - 전환 중에 강제 종료 뒤 곧바로 재연결하는 규칙(§7.1)을 추가했다. Swift 앱에 먼저 넣은 뒤 판단 코어가 같은 규칙을 구현했다.
- **함께 한 정리**
  - IPv4 읽기, 인증서 값 형식 확인, 인증서 값 비우기 규칙을 각각 한 곳으로 모았다.
  - 검증된 버전 판단은 UI에서 backend adapter로, "다음 연결부터 적용"과 상태 줄 판단은 UI에서 판단 코어로 옮겼다.
  - 준비 상태의 "Gmail 앱 비밀번호 있음"을 실제 뜻인 "Gmail 인증 있음"(앱 비밀번호 또는 Google 계정)으로 바꿨다.
  - OTP 세션의 시도 기록을 런타임의 OTP 세션 하나로 모았다.
  - 테스트에만 쓰던 시계 주입, config.yaml 통째 저장·읽기 전용 불러오기, 큰 스택 스레드는 옮기지 않았다.
- **결과**: Helper 쪽 설치 뒤 강제 종료부터 재연결까지 openfortivpn 11.1초, Helper 7.1초(전에는 19.4초)가 걸렸다. 앱 쪽 전환 설치 뒤에는 앱 종료부터 터널 연결까지 약 11초였다. §10의 점검 가운데 2·3·4·8·9·10과 7의 일부는 실제 환경에서 확인했다. 6은 사용자 결정으로, 1은 openfortivpn 설정 파일의 비밀번호가 Keychain을 덮어쓸 수 있어 자동 테스트로 대신했다. 5(잠자기)는 사용자가 나중에 확인한다.

## 8. 보안 요약과 감수하는 위험
- 비밀값은 Keychain에 저장하고, Helper에는 XPC로 메모리 안에서만 전달한다. argv, 설정 파일, 환경 변수, 로그에 남지 않는다. Rust에서 비밀값을 담는 타입은 디버그 출력에서 값을 가린다. panic 메시지와 crash report에 들어갈 수 있기 때문이다.
- XPC는 양쪽 모두 Team ID와 signing identifier를 확인한다. Helper는 검증된 값만 받고, config.yaml은 읽지 않는다.
- **감수하는 위험** (개인 Mac, 사용자 결정)
  1. **서버 인증서 자동 신뢰**: 공용 Wi-Fi 등에서 능동적인 MITM 공격자가 가짜 인증서를 내밀면 자동으로 신뢰한다. 그러면 VPN 비밀번호와 실시간 OTP가 넘어갈 수 있다. OTP를 자동 제출하므로 사람이 알아챌 틈이 없다.
  2. **같은 사용자 권한의 코드가 root로 이어지는 경로**: Homebrew 바이너리 변조, Personal Team 키를 이용한 앱 사칭, `/Applications`의 앱 바꿔치기가 가능하다.
  3. **config.yaml 변조**: `vpn.host`를 바꾸면 1번과 결합해 자격증명이 다른 서버로 갈 수 있다. Keychain 항목이 `username@host` 단위라 host가 바뀌면 비밀번호를 다시 입력해야 하므로 부분적으로 완화된다.
- 강화 옵션(인증서 변경 시 확인, root 소유 바이너리 사본, 게이트웨이 고정과 관리자 인증)은 추후 과제다.
- **Windows**
  - 비밀값은 Credential Manager에 저장하고, Helper에는 named pipe로 메모리 안에서만 전달한다.
  - openconnect는 관리자만 쓸 수 있는 Program Files의 것만, logout 요청용 curl은 System32의 것만 실행한다.
  - 세션 cookie는 Helper 메모리에만 두고, openconnect와 curl에는 stdin으로 넘긴다. 로그에는 값을 남기지 않는다.
  - 감수하는 위험: 서명 확인이 없어서, 같은 사용자로 실행되는 다른 프로그램도 Helper에 연결해 연결, 해제, 예외 route 쓰기를 요청할 수 있다. Helper는 받은 값을 모두 다시 검증하므로 요청할 수 있는 일은 앱으로 할 수 있는 일로 한정된다. 위의 1·3번 위험은 Windows에도 같다.

## 9. 리스크와 추후 과제
- FortiOS 7.6.3 이상으로 게이트웨이를 올리면 openfortivpn을 쓸 수 없다. openconnect의 `--protocol=fortinet`도 같은 SSL VPN tunnel mode라 대안이 되지 않는다. 대안은 아직 없다.
- 잠자기 중 PPP kernel extension panic 보고가 있다(openfortivpn #1278).
- health check가 프로세스와 interface로 한정돼 있어 서버 idle-timeout으로 죽은 연결은 늦게 감지될 수 있다.
- Personal Team 인증서는 1년마다 만료된다.
- openfortivpn을 업그레이드하면 출력 문자열이 바뀔 수 있다. 검증된 버전이 아니면 고급 pane에 경고를 띄운다.
- Windows에서 openconnect를 쓸 때의 경고(§6)는 알려진 문제로 두고 써 보면서 개선한다. driver는 Wintun을 기본으로 하고, 불안정하면 TAP을 검토한다.
- Windows는 터널 뒤 요청 없는 종료를 모두 강제 종료로 보므로, 접속 직후 스스로 끊기는 일이 반복되면 두 번째에 멈추지 않고 시도 상한(한 시간 12번)까지 로그인과 OTP 메일이 반복된다.
- 회사 보안 프로그램이 서비스가 실행하는 PowerShell을 막으면 Windows 예외 규칙과 게이트웨이 host route 정리가 동작하지 않는다.
- Windows에서 openconnect를 늘 강제로 끝내므로, Helper가 죽어 세션 cookie를 잃으면 그 세션은 idle timeout(50분)까지 서버에 남는다. 지금은 같은 계정의 동시 세션이 허용돼 다음 연결에 지장이 없다. 회사가 동시 접속을 막으면 다시 봐야 한다.
- logout 요청의 응답은 세션이 있든 없든 같아서, Helper는 logout이 됐는지 확인하지 못한다.
- 같은 사용자의 다른 프로그램이 Helper 서비스보다 먼저 같은 이름의 통로를 만들면 Helper는 통로를 열지 못하고 서비스가 시작에 실패한다. 앱은 그 통로의 소유자가 시스템 계정이 아니어서 비밀번호를 보내지 않는다.
- 회사 보안 프로그램(AhnLab MDS)은 처음 보는 실행 파일을 분석해 막는다. Helper 서비스도 다른 프로그램(openconnect, curl, PowerShell, taskkill)을 띄우므로 막힐 수 있다. 그러면 보안 담당 부서의 예외 등록이 필요하다.
- 아직 확인하지 않은 Windows 동작: route 스크립트가 openconnect의 10초 제한을 넘긴 뒤에도 route가 다 들어가는지, 살아 있는 세션 cookie로 세션 정보를 물었을 때 접근 거부(HTTP 403)가 나온 이유.
- 추후 과제
  - Linux 앱(Rust + QML)과 서비스
  - Windows 빌드 환경과 CI(GitHub Actions 후보, 사용자 확인 필요)
  - macOS와 Linux의 openconnect: 실행 파일 위치, 환경 변수, 터널 어댑터 줄, 정상 종료 요청(SIGTERM)과 그 뒤의 logout 규칙 확인
  - 앱 크기 줄이기
  - 지인 배포, 인증서 보안 강화, split DNS, 다중 프로필, 비정상 종료 뒤 터널 이어받기(adopt), root 소유 바이너리 사본

## 10. 수동 점검
큰 변경 뒤 실제 환경에서 확인할 시나리오다.
1. config.yaml이 없는 상태로 첫 실행: 기존 openfortivpn 설정을 가져오고, Helper 승인과 Gmail 연결 후 openfortivpn 1회 실행으로 연결된다. openfortivpn 설정에 비밀번호가 있으면 Keychain의 VPN 비밀번호를 그 값으로 덮어쓰므로, 값이 최신일 때만 한다.
2. `vpn.trusted_certs`를 비우고 연결: 인증서 거부 1회 뒤 즉시 재실행으로 연결되고 digest가 저장된다. 다음 연결부터는 거부 없이 1회에 연결된다.
3. openfortivpn 또는 pppd를 강제로 종료: route가 복구되고 재연결된다. openfortivpn을 SIGKILL로 끝내면 곧바로, pppd만 끝내면 서버 종료처럼 잠시 뒤 재연결한다. 직전 연결 후 2분 안에 끝내면 너무 빨리 끊긴 세션으로 보고 더 오래 기다린다.
4. 세션 최대 시간을 짧게 바꿔 사전 재연결이 동작하는지 확인한다.
5. 짧은 잠자기, 긴 잠자기, 연결 중 잠자기, Wi-Fi 변경
6. 잘못된 VPN 비밀번호, 잘못된 Gmail 인증, OTP 메일 차단: 각각 정해진 대로 멈춘다.
7. config.yaml 직접 편집: 옵션 변경 반영, 잘못된 값 거부, 깨진 YAML에서 마지막 정상 설정 유지, symlink 유지
8. 앱 강제 종료와 정상 종료: 프로세스가 정리되고 route가 복원된다.
9. 보안: `ps`와 로그에 비밀값이 없고, 렌더링된 openfortivpn 설정 파일은 0600이며 종료 후 삭제된다. config.yaml과 state.json은 0600이다. `make verify`가 통과한다.
10. Helper가 터널 연결 중 죽었을 때: 터널이 올라온 상태에서 `sudo kill -9`로 Helper를 끝낸다. 앱이 곧바로 재연결을 시작하고, 새 Helper가 이전 Helper가 남긴 프로세스를 멈추고 끝나지 않은 실행을 정리했다는 로그를 남긴다. 남아 있던 openfortivpn과 pppd가 사라지고 새 터널이 올라온다.

**Windows** (위 시나리오 중 OS와 무관한 4·6·7·9는 그대로 하고, 다음을 더한다)
1. Mac의 config.yaml을 복사한 첫 실행: 비밀번호와 Gmail 인증만 넣으면 연결된다. `vpn.trusted_cert_pins`가 비어 있으면 거부 1회 뒤 즉시 재실행으로 연결되고 값이 저장된다.
2. [연결 해제]와 사전 재연결: 로그에 강제 종료와 logout 요청(응답 코드)이 남고, 게이트웨이 host route가 사라지고, 남은 openconnect·route 스크립트 프로세스가 없다.
3. openconnect를 작업 관리자로 끝냄: 게이트웨이 host route가 정리되고 곧바로 재연결된다.
4. Helper 서비스를 작업 관리자로 끝냄: Windows가 서비스를 다시 시작하고, 새 서비스가 남은 openconnect를 정리한 뒤 앱이 다시 연결한다.
5. 예외 규칙: 유선과 Wi-Fi 각각에서 대상이 직접 경로로 가고, VPN 재연결 뒤에도 유지된다.
6. 네트워크를 잠깐 끊었다가 되돌림: openconnect가 곧바로 끝나고, 네트워크가 돌아오면 다시 연결된다. 절전과 재개
7. 보안: 작업 관리자의 명령줄과 로그에 비밀값이 없다.
8. 일반 사용자 계정: 관리자 계정 정보를 넣어 설치한 뒤, 일반 사용자의 앱이 서비스에 연결된다.
