# FortiVPNManager 설계안 (v1)

## 1. Context
- FortiGate SSL VPN 서버가 약 3시간마다 세션을 강제로 끊는다. 그때마다 `sudo openfortivpn`을 다시 실행하고 Gmail로 온 OTP를 손으로 입력해야 한다.
- 목표: macOS 메뉴바 상주 앱이 아래를 대신한다.
  - openfortivpn 구동
  - Gmail(Google Workspace) IMAP에서 OTP를 자동으로 읽어 인증
  - 끊기거나 만료 시각이 다가오면 자동 재연결
- 환경
  - macOS 27.0.1, Apple Silicon. v1은 본인 Mac 전용이며 유료 Apple 개발자 계정은 없다.
  - Xcode 27.0이 설치돼 있다. `xcode-select`는 아직 CLT를 가리키고, 서명 인증서는 0개다.
  - openfortivpn 1.24.1(Homebrew), `/usr/sbin/pppd` 2.4.2(Apple).
  - AhnLab EDR이 설치돼 있다.
  - 게이트웨이 인증서는 Fortinet_Factory(공장 기본, 2056년까지 유효)다. 공인 CA 인증서가 아니므로 `trusted-cert` 핀이 있어야 접속된다.
- 시작점은 빈 디렉토리다.
- 기존 `~/.config/openfortivpn/config`(권한 644)에는 비밀번호가 평문으로 들어 있다. 이 파일의 `insecure-cert = 1`은 유효하지 않은 키지만, 마지막 줄이라 지금까지 동작에 영향이 없었다.

## 2. 확정 결정
| 항목 | 결정 |
|---|---|
| 재연결 | 사후 재연결(openfortivpn 종료 감지)과 선제 재연결(세션 최대시간 − 여유분)을 함께 쓴다 |
| 권한 | Privileged Helper. SMAppService.daemon + XPC를 쓰고, Personal Team(무료 Apple ID) "Apple Development" 인증서로 서명한다. 스파이크가 실패하면 레거시 LaunchDaemon(최초 1회 관리자 암호)으로 대체한다 |
| 스택 | Swift 6, SwiftUI(메뉴·뷰), AppKit 창. Xcode + XcodeGen에 로컬 SwiftPM 패키지를 붙인다. YAML은 Yams 6.2.x |
| 번들 ID | 앱 `com.qoor.FortiVPNManager`, 헬퍼 `com.qoor.FortiVPNManager.Helper`(launchd Label과 Mach 서비스 이름도 같다). **최초 register 이후에는 바꾸지 않는다** |
| openfortivpn | Homebrew 경로를 그대로 쓴다. 개인 Mac이라 위험을 감수한다 |
| 서버 인증서 | **무조건 자동 신뢰.** 사용자가 신경 쓸 필요가 없게 UI에서 뺀다. 처음 확인한 digest는 **config.yaml의 자동 관리 필드 `vpn.trusted_certs`에 저장**하고, 이후 매 연결에 `trusted-cert`로 넘긴다. 그래서 평소에는 openfortivpn을 1회만 실행한다. 2회 실행은 처음 연결할 때와 인증서가 바뀌었을 때뿐이다. digest는 openfortivpn이 거부할 때 출력하는 값을 읽어 얻는다. 앱이 TLS나 해시를 직접 계산하지 않는다 |
| OTP | **Gmail API(OAuth, 권장)**: 조직 내 Google Cloud 프로젝트의 데스크톱 OAuth 클라이언트(Internal, `gmail.modify`) + PKCE 루프백 로그인, history.list 0.5초 폴링으로 메일이 Gmail에 들어오는 즉시 감지. OAuth 클라이언트는 gitignore된 `Config/GoogleOAuthClient.json`을 빌드 시 앱 번들에 내장하고(데스크톱 클라이언트 secret은 비밀로 취급되지 않음, PKCE로 보호), 사용자는 'Google 계정 연결'만 한다. 다른 클라이언트는 계정 탭 '고급'에서 JSON으로 가져와 덮어쓸 수 있다. **대체: 앱 비밀번호 + IMAP IDLE**(API 인증 실패 시 같은 시도 안에서 자동 전환). 사용한 OTP 메일은 휴지통으로 옮긴다. 실측: IMAP은 Gmail 도착 후 IMAP 노출까지 약 7초 지연 |
| 설정 저장 | `~/.config/FortiVPNManager/config.yaml`(비밀이 아닌 설정). 비밀번호 2종은 Keychain. 자세한 내용은 5.7절 |
| 잠자기 | 터널을 유지하고, 깨어나면 6.3절 규칙으로 판단한다 |
| 헬스체크 | 프로세스와 ppp 인터페이스만 감시한다. 별도 host 설정은 두지 않는다 |
| 실패 정책 | 보수적으로 재시도한다. 수동 OTP 입력창은 만들지 않는다 |
| UI 용어 | 화면에 표시하는 문구에서는 "헬퍼" 대신 **"Helper"**를 쓴다 |
| 언어 정책 | 소스 코드·주석·앱/Helper가 남기는 로그(로그 탭 `▸` 메타 줄 포함)는 **영어**. openfortivpn/pppd 원본 출력은 그대로. UI 문구는 코드에 영어로 쓰고 한국어 번역은 `App/Resources/Localizable.xcstrings`(String Catalog)에 둔다 |
| 메뉴바 | 아이콘은 연결됨 / 끊김 / 연결·재연결 중 / 오류만 보여준다. 알림은 v1에서 제외한다 |
| 설정창 | 탭 3개: **계정**(VPN + Gmail OTP), **연결 관리**(연결 옵션 + Helper), **로그**(openfortivpn stdout/stderr 원본) |
| 로그 | 마스킹 없이 원본을 보여준다. 기본 verbosity에서는 비밀번호, OTP, 쿠키가 출력되지 않음을 소스로 확인했다 |
| 기타 | 로그인 시 자동 실행하고 앱 시작 시 자동 연결한다. DNS와 라우팅은 openfortivpn 기본 동작을 따른다. 사용자가 마스킹한 실제 로그와 OTP 메일 샘플을 제공한다. 지인 배포는 추후 과제로 둔다 |

## 3. 검증된 핵심 제약 (openfortivpn 1.24.1 소스 + 실측)
1. **`--persistent`는 쓸 수 없다.** 이메일 OTP 경로에서는 이미 쓴 OTP를 다시 보낸다. 그래서 **연결할 때마다 새 프로세스를 띄운다.**
2. **비밀값은 stdin 파이프로 주입한다.**
   - `"VPN account password: "` 프롬프트는 프로세스 시작 직후, TLS 연결 전에 나온다(main.c:732).
   - OTP 프롬프트는 두 가지다.
     - tokeninfo 경로: `"Two-factor authentication token: "`
     - HTML 폼 경로: `"Please…"`(서버 문구) 또는 기본값 `"Please enter one-time password: "`
     - 사용자 게이트웨이가 어느 경로인지는 Phase 0b에서 확인한다.
   - 프롬프트에는 **줄바꿈이 없다.** 그래서 프롬프트 감지는 줄이 끝나기를 기다리지 않고, 아직 끝나지 않은 꼬리를 즉시 검사하는 별도 경로로 처리한다. 비밀번호를 쓴 직후 나오는 개행은 다음 로그 줄과 붙어서 나온다.
   - 입력값은 출력에 되찍히지 않는다. **PTY를 쓰지 않고 파이프를 고정한다.**
3. **출력 스트림의 구성**
   - stdout: openfortivpn 자체 로그 전부(`INFO:   `/`WARN:   `/`ERROR:  ` 8자 접두어, 타임스탬프와 ANSI 색 없음, 줄마다 flush). `/sbin/route` 출력(접두어 없음)도 여기로 온다.
   - stderr: pppd 메시지(`Fri Apr  5 11:31:24 2024 : …` 형식의 타임스탬프 포함), route 오류, 드문 내부 오류.
   - **stderr가 곧 오류라는 뜻은 아니다.**
4. **비밀값이 출력되는 조건**: 쿠키는 `-v`부터, stdin으로 넣은 OTP는 `-vv`부터 출력된다. 비밀번호는 `-vvv`에서만 출력된다. **기본 verbosity로 고정하면 원본 로그가 안전하다.** 유일한 예외는 `Received bad header` 덤프로, 터널 트래픽 조각이 최대 256B까지 출력된다.
5. **종료 원인을 출력만으로 구분할 수 없다.**
   - 종료 코드는 0, 1, 254가 원인과 무관하게 섞여 나온다.
   - `pppd: The link was terminated by the modem hanging up.`는 본문이 같고 접두어만 상황에 따라 INFO/ERROR로 바뀐다. 이 구분도 pppd 종료코드에 따라 달라지고 macOS 27에서 실측되지 않았다.
   - 그래서 원인은 **헬퍼가 기록한 사실**로 판단한다(5.1절 ExitFacts).
   - set-dns 기본값이 1이라서, 연결 해제 시 `Removing VPN nameservers...`가 출력된다.
6. **인증서 검증 동작** (tunnel.c:857-938)
   - 검증 순서: ① 시스템 CA 검증 + 호스트 일치 → ② 실패하면 leaf DER의 SHA-256(소문자 hex 64자)을 `trusted-cert` 목록과 비교 → ③ 그래도 아니면 거부.
   - **검증을 끄는 옵션은 없다.** `--insecure-ssl`은 프로토콜과 cipher 제한만 푼다.
   - 거부할 때 stdout에 `ERROR:      trusted-cert = <digest>`를 출력한다. 같은 블록이 두 번 이상 나올 수 있다.
   - 이 블록은 인증 이후 단계(VPN 할당 뒤의 두 번째 TLS 연결 등)에서도 나올 수 있다.
   - `trusted-cert`는 여러 줄 지정할 수 있다.
   - 비밀번호가 네트워크로 나가는 것은 TLS 검증을 통과한 다음이다. 따라서 인증서 거부로 끝난 시도는 로그인 시도로 집계되지 않는다(잠금과 무관).
7. **시그널 처리**
   - SIGTERM 핸들러는 터널이 올라온 뒤(io_loop)에만 설치되며, 이후 SIGTERM으로 라우트를 복원하고 로그아웃한다.
   - 그 전에는 즉시 종료된다. 단, spawn할 때 시그널 마스크와 디스포지션을 초기화해야 이렇게 동작한다.
   - SIGHUP과 SIGPIPE는 무시한다.
8. **비정상 종료 시 기본 라우트가 사라진다.** 그래서 RouteRepair가 필요하다.
9. **연결이 죽었는지 감지하기 어렵다.** Apple pppd에는 LCP echo가 없고, openfortivpn에는 네트워크 타임아웃이 없다. 그래서 **단계마다 워치독을 둔다.**
10. **OTP 메일과 서버 정책**
    - 기본 제목은 `AuthCode: NNNNNN`이고, 발신자는 일정하지 않다. 유효시간은 기본 60초다.
    - `UID SEARCH UID n:*`는 n보다 작은 마지막 메일도 돌려준다. 그래서 클라이언트에서 한 번 더 걸러야 한다.
    - `\All`에는 스팸함이 포함되지 않는다.
    - FortiGate 잠금 정책: 같은 IP에서 2회 실패하면 60초 차단된다. `login-timeout`은 30초다.
11. **openfortivpn은 `https_proxy`/`all_proxy` 환경 변수를 읽는다.** 그래서 헬퍼는 PATH 외의 환경 변수를 넘기지 않는다(불변식).
12. **서명 주의**: Personal Team Development 서명은 Release 빌드에도 `get-task-allow`를 주입한다. App Sandbox나 제한 entitlement를 쓰면 7일짜리 프로비저닝 프로파일이 붙으므로 **entitlement는 비워 둔다.**

## 4. 아키텍처
```
┌──────────── FortiVPNManager.app (사용자 권한, LSUIElement) ────────────┐
│ MenuBarExtra(.menu) / SettingsWindowController(NSWindow: 계정·연결 관리·로그)│
│ ConnectionController(@MainActor) ── FortiCore.ConnectionReducer (순수)  │
│   ├ Readiness  ├ ReconnectScheduler  ├ SystemMonitor  ├ GmailOTPSession │
│   ├ HelperClient(XPC, event/output → 순서 보장 단일 채널)                │
│   └ LogStore(링버퍼) → LogFileWriter(~/Library/Logs/…)                   │
│ ConfigStore(config.yaml 감시·검증) / KeychainStore / StateStore(state.json)│
└───────────────────────────────┬─────────────────────────────────────────┘
                  NSXPCConnection (양방향 코드서명 요건: anchor apple generic + Team OU + identifier)
┌──────────── FortiVPNManagerHelper (root LaunchDaemon, 수요 기동) ────────┐
│ XPCService → FortiHelperCore.TunnelSupervisor                           │
│   posix_spawn(openfortivpn -c <conf>) · stdin 주입 · 프롬프트 감지       │
│   LineAssembler(stdout/stderr) → output 배치 전송 · 워치독               │
│   SIGTERM→10s→SIGKILL · RouteRepair · pppd 정밀 정리 · pidfile           │
│ BinaryLocator(Homebrew 경로 해석·기본 검사·버전 확인)                     │
└─────────────────────────────────────────────────────────────────────────┘
```
- **역할 분담**: 상태머신, OTP 처리, 스케줄링, 설정 파일, 인증서 핀은 앱이 맡는다. 헬퍼는 실행·감시·정리만 하고 상태를 저장하지 않으며 config.yaml도 읽지 않는다.
- **크래시 후 재연결(adopt)은 v1에서 하지 않는다.** 제어 XPC 연결이 끊기면 헬퍼가 터널을 정상 종료한다. 따라서 Quit, 강제 종료, 크래시는 모두 같은 경로를 탄다.

## 5. 컴포넌트

### 5.1 FortiShared (XPC 계약)
- **메서드**
  - `getVersion() → BuildID`
  - `connect(attemptID, ConnectRequest{host, port, username, realm?, trustedCerts: [String], password})`
  - `submitOTP(attemptID, code)`
  - `disconnect(attemptID?)`
  - `status()`
  - `exit()`
- **콜백**: 둘 다 attemptID를 포함한다.
  - `event(HelperEvent)`
    - `.milestone(.spawned | .connectedToGateway | .otpPrompted | .otpSubmitted | .authenticated | .tunnelUp(ip))`
    - `.exited(ExitFacts)`
  - `output(Data)`: 로그 줄 배치. 각 줄은 `{seq, date, stream: stdout|stderr|helper, text}`.
- **`ExitFacts`**: `furthestMilestone`, `stopInitiator(helper|none)`, `watchdogFired(phase?)`, `certRejectedDigests: [String]`, `authFailLineSeen`, `upDuration`, `exitStatus`, `routeRepairPerformed`, `lastOutputSeq`.
- **순서 규칙**
  - event와 output은 같은 연결에서 FIFO로 전달된다. reply는 이들과 순서가 보장되지 않는다.
  - 그래서 **해제나 Quit의 완료는 reply가 아니라 `.exited`로 판정한다.**
  - 앱의 nonisolated sink는 메시지를 받는 즉시 하나의 AsyncStream에 동기적으로 yield한다. 메시지마다 `Task{}`를 만들지 않는다.
- **전송 형식**: Codable을 `Data`로 감싼다. reply는 `@escaping @Sendable`이다. 호출은 `remoteObjectProxyWithErrorHandler`와 resume-once 가드를 둔 continuation으로 감싼다.

### 5.2 Helper (root)
- **XPC 보안**
  - 리스너는 `setConnectionCodeSigningRequirement("anchor apple generic and identifier \"com.qoor.FortiVPNManager\" and certificate leaf[subject.OU] = \"<TEAMID>\"")`을 건다.
  - TEAMID는 런타임에 자기 자신의 서명에서 읽는다. 읽지 못하면 거부한다.
  - get-task-allow 조건은 넣지 않는다. 같은 사용자 권한 위협은 이미 감수했기 때문이다.
  - 앱 쪽도 `resume()` 전에 헬퍼 요건을 건다. 비밀번호는 `getVersion` 응답을 받은 뒤에만 보낸다.
- **제어권**
  - 연결한 쪽의 uid를 audit token으로 기록한다.
  - 같은 uid에서 새 연결이 오면 기존 연결을 대체한다.
  - 연결이 무효화되면 즉시 정상 종료한다.
  - 자식 프로세스가 살아 있는 동안에는 connect를 거부한다.
- **설정 렌더러** (`FortiCore.ConfigRenderer`)
  - 허용 키는 `host`, `port`, `username`, `realm`, `trusted-cert`(여러 줄)뿐이다.
  - 바이트 제한: host 253, username 64, realm 63.
  - host는 호스트명 또는 IPv4만 허용한다.
  - trusted-cert는 각각 `^[0-9a-f]{64}$`이어야 한다.
  - 앞뒤 공백, CR/LF/NUL/제어문자는 거부한다.
  - 파일은 `/var/run/com.qoor.FortiVPNManager.Helper/`(0700) 아래에 mkstemp(0600)로 만들고 종료하면 삭제한다.
- **BinaryLocator**
  - `/opt/homebrew/bin/openfortivpn`의 realpath가 `/opt/homebrew/Cellar/openfortivpn/` 아래에 있는 일반 파일이어야 하고, world-writable이 아니어야 한다.
  - 버전이 1.24.x가 아니면 "연결 관리" 탭에 경고를 표시한다.
- **spawn** (`FortiHelperCore.ProcessRunner`)
  - 인자는 `[path, "-c", conf]`로 고정한다.
  - 환경은 `PATH=/usr/bin:/bin:/usr/sbin:/sbin` 하나뿐이다.
  - `posix_spawn` 플래그: `SETSIGMASK`(빈 집합), `SETSIGDEF`(잡을 수 있는 모든 시그널), `CLOEXEC_DEFAULT`, `SETPGROUP`.
  - 표준 입출력은 `adddup2`로 fd 0/1/2만 연결한다.
  - 헬퍼 자신은 SIGPIPE를 무시하고, EPIPE는 자식이 종료된 것으로 처리한다.
  - 헬퍼의 SIGTERM은 DispatchSource로 받아서 자식을 정상 종료시킨다.
  - 자식 종료는 DispatchSource proc 이벤트와 waitpid로 감지한다.
- **입력 주입**
  - 비밀번호: `Connected to gateway.` 전에 나온 첫 프롬프트에만 한 번 쓴다. CR/LF/NUL은 거부하고 256B 미만이어야 한다. 쓴 뒤에는 메모리에서 지운다.
  - OTP: `otpPrompted` 단계에서만, `^[0-9]{6,8}$`일 때 한 번 쓴다. 쓴 직후 stdin을 닫는다.
- **출력 처리**
  - 두 파이프를 같은 시리얼 큐에서 읽는다.
  - 프롬프트 감지는 끝나지 않은 꼬리를 즉시 검사하는 별도 경로다.
  - 화면용 `FortiCore.LineAssembler`(스트림별) 규칙
    - LF로 자르고, 끝의 CR은 제거한다.
    - 250ms 동안 입력이 없으면 부분 줄을 내보낸다.
    - 이미 내보낸 부분 줄을 끝내는 LF가 단독으로 오면 무시한다.
    - 한 줄은 16KB를 넘지 않게 자른다.
    - UTF-8로 디코딩할 수 없는 바이트는 대체 문자로 바꾼다.
    - 제어문자는 `^X` 형태로 표기한다.
  - **마스킹하지 않는다.**
  - 헬퍼 자신의 메타 줄(`stream: helper`)도 넣는다: 시도 시작/pid, `‹password sent›`, `‹OTP sent›`, 보낸 시그널과 그 사유, 종료 정보(모두 영어).
  - 전송은 100ms 단위 배치로 한다. `.exited`를 보내기 전에 남은 출력을 반드시 flush한다. 종료 후 남은 출력을 기다리는 시간은 최대 1초다.
- **워치독** (`ContinuousClock` 기준)

  | 구간 | 제한 |
  |---|---|
  | spawn → `Connected to gateway.` | 30s |
  | `Connected to gateway.` → OTP 프롬프트 또는 `Authenticated.` | 30s |
  | OTP 프롬프트 → OTP 제출 | 앱 50s, 헬퍼 보조 65s |
  | OTP 제출 → `Authenticated.` | 20s |
  | `Authenticated.` → `Tunnel is up and running.` | 40s |
  | 시도 전체 | 150s (앱이 관리) |
  | 종료 요청 → 실제 종료 | SIGTERM 후 10s 지나면 SIGKILL |

- **RouteRepair**: 라우트를 건드렸는데(routesTouched) 복원하지 못한 채(!routesRestored) 종료하면 다음을 수행한다.
  - `State:/Network/Global/IPv4`의 Router로 기본 라우트를 복구한다.
  - 게이트웨이 /32 라우트를 삭제한다.
- **고아 정리**
  - pidfile에 openfortivpn과 pppd의 (pid, 시작시각)을 기록한다.
  - 헬퍼가 시작될 때 정리 대상은 (pid, 시작시각)이 기록과 일치하고, 경로가 일치하며, uid가 0인 프로세스뿐이다.
  - 정리 순서는 SIGTERM, SIGKILL, 그다음 RouteRepair다.
- **헬퍼 운영 로그**는 os.Logger(subsystem `com.qoor.FortiVPNManager`)에 남긴다.

### 5.3 GmailOTPSession (시도마다 IMAP 세션 하나)
- Network.framework TLS 위에 최소한의 IMAP 클라이언트를 직접 구현한다.
- `FortiCore.IMAPResponseParser`는 재개 가능한 바이트 상태머신이다.
- 처리 순서
  1. **연결을 시작하기 전에** 다음을 한다.
     - `AUTHENTICATE PLAIN`(SASL-IR)으로 로그인한다.
     - `LIST` SPECIAL-USE로 `\All`과 `\Trash` 메일함을 찾는다.
     - `\All`을 SELECT하고 `UIDVALIDITY`/`UIDNEXT`를 스냅샷한다.
     - 스냅샷 이후 UID에 `AuthCode` 메일이 이미 있으면 휴지통으로 옮긴다.
  2. IDLE로 대기한다. 5초마다 `DONE` 후 `UID SEARCH`로 보조 확인을 한다.
  3. `otpPrompted`가 오면 후보 메일을 고른다. 조건은 다음을 모두 만족해야 한다.
     - UID가 스냅샷 이상이다(클라이언트에서 거른다).
     - `UIDVALIDITY`가 변하지 않았다.
     - `INTERNALDATE`가 `otpPromptedAt − 5s` 이후다.
     - 제목이 `AuthCode:\s*(\d{6,8})`에 맞는다. RFC 2047 인코딩은 필요할 때만 디코딩하고, 헤더만 PEEK로 가져온다.
     - 후보가 여럿이면 가장 큰 UID를 쓴다.
  4. `submitOTP`를 보낸 뒤 후보 메일을 전부 `UID MOVE`로 `\Trash`에 옮기고 `LOGOUT`한다.
  5. 50초 마감까지 코드를 못 찾으면 제출하지 않고 중단한다. 이때 `\Junk`를 한 번 검색해, 메일이 스팸함에 있으면 실패 사유에 함께 표시한다.
- 제목 패턴은 코드 상수로 둔다. Phase 0b에서 받은 실제 메일 샘플로 확정한다.

### 5.4 ConnectionController / ConnectionReducer
- 리듀서 형태는 `(State, Event) → (State, [Effect])`다. attemptID가 다르거나 timer generation이 다른 입력은 무시한다.
- 상태: `idle`, `needsSetup(missing)`, `connecting(attemptID, reason, milestone)`, `connected(since, ip, plannedReconnectAt, underlay)`, `waitingRetry(until, reason)`, `waitingNetwork`, `disconnecting`, `failed(reason)`.
- `desiredState`는 실제 상태와 분리해서 관리한다.
- **Readiness**: 연결 전에 항상 확인한다.
  - 확인 항목
    - config.yaml이 유효한가
    - Keychain에 VPN 비밀번호(`username@host`)와 Gmail 앱 비밀번호(`gmail_address`)가 있는가
    - 헬퍼가 `.enabled` 상태인가
    - BuildID가 일치하는가
  - 하나라도 부족하면 `needsSetup`으로 간다.
  - 재확인 시점: 앱 활성화, 설정창 열기, 시스템 설정을 연 뒤 2분간 2초 간격, config.yaml이 바뀌었을 때.
- **서버 인증서 자동 신뢰** (사용자 개입 없음)
  - 매 연결마다 config.yaml의 `vpn.trusted_certs`를 ConnectRequest의 `trustedCerts`로 넘긴다. 평소에는 이 값으로 바로 통과하므로 **openfortivpn을 1회만 실행한다.**
  - 처음 연결할 때 이 필드가 비어 있거나 서버 인증서가 바뀐 경우, `.exited`에 `certRejectedDigests`가 담겨 온다. 이 판단은 인증 실패로 분류하기 **전에** 한다.
    1. 받은 digest를 `vpn.trusted_certs`의 맨 앞에 추가하고 즉시 config.yaml에 저장한다. 최대 4개까지 두고 넘치면 가장 오래된 것을 지운다.
    2. 이 시도 체인에서 처음 일어난 거부라면 IMAP 세션을 닫고 **즉시 다시 시도**한다. 인증 실패 카운트는 올리지 않는다. TLS 단계에서 끝난 시도라 비밀번호가 전송되지 않았고, 잠금 정책과도 상관없다.
    3. 같은 체인에서 또 거부되면 네트워크 오류로 분류한다.
  - `otpSubmitted` 이후에 거부되면, digest는 똑같이 저장하되 6.2절의 "OTP 사용 후 실패" 규칙을 따른다.
  - 처음 연결하기 전에 "기존 openfortivpn 설정 불러오기"로 `trusted-cert`를 가져와 두면 첫 연결도 1회 실행으로 끝난다.
  - 설정창에서 `vpn.host`나 `vpn.port`를 바꾸면 `trusted_certs`를 비운다. 다음 연결에서 한 번만 자동으로 다시 수집한다.
- **선제 재연결**
  - `plannedReconnectAt = connectedAt + session_max_minutes − margin_minutes`로 계산한다.
  - `Date`로 저장하고, 깨어났을 때 다시 판단한다.
  - 기존 세션을 먼저 끊고(break-before-make) 새로 연결한다.
- 연결 중에는 `beginActivity(.userInitiatedAllowingIdleSystemSleep)`를 유지한다.

### 5.5 SystemMonitor
- 감시 대상: `NSWorkspace.shared.notificationCenter`의 willSleep/didWake, `NWPathMonitor`, `getifaddrs`의 ppp* 인터페이스(10초 간격).
- underlay는 tunnelUp 시점의 `route -n get <gatewayIP>` 결과(인터페이스, source IP)로 정한다. ppp*, utun*, `.other`는 제외한다.
- 연결 중이거나 해제 중일 때 들어온 경로 이벤트는 무시한다.

### 5.6 UI
- **아이콘**

  | 아이콘 | 상태 |
  |---|---|
  | 연결됨 | `connected` |
  | 끊김 | `idle` |
  | 연결·재연결 중 | `connecting`, `waitingRetry`, `waitingNetwork`, `disconnecting` |
  | 오류 | `failed`, `needsSetup` |

- **메뉴**
  1. 상태 텍스트(30초마다 갱신)
  2. `연결`: desired가 꺼져 있거나 `failed`/`needsSetup`일 때 보인다. 누르면 실패 카운터를 초기화한다.
  3. `연결 해제`: desired가 켜져 있을 때 보인다.
  4. `지금 재연결`: `connected`/`waitingRetry`에서 누를 수 있다. 마지막 인증 실패 후 90초가 지나지 않았으면 그때까지 기다린다.
  5. `설정…` (⌘,)
  6. `종료`: `.terminateLater`로 처리하고, `.exited`가 오거나 최대 12초가 지나면 종료한다.
  - 오류이거나 설정이 필요할 때는 사유 한 줄과 "설정 열기"를 함께 보여준다.
- **설정창 공통**
  - SwiftUI Settings 씬을 쓰지 않는다.
  - `SettingsWindowController`가 `NSWindow`(`isReleasedWhenClosed = false`)와 `NSHostingController`, TabView로 창을 만든다.
  - 창을 열 때는 `DispatchQueue.main.async`로 미룬 뒤 `.regular` → activate → `makeKeyAndOrderFront` 순으로 호출하고, 닫으면 `.accessory`로 되돌린다.
  - NSWindow 서브클래스의 `performKeyEquivalent`에서 cut/copy/paste/selectAll/undo와 ⌘F를 처리한다.
- **탭 ① 계정** (한 화면, 그룹 두 개)
  - **VPN**: 서버 주소, 포트, 사용자명, 비밀번호, realm(접힌 "고급" 안).
  - **2단계 인증(Gmail)**: Gmail 주소, 앱 비밀번호, [Gmail 연결 테스트](로그인과 `\All`/`\Trash` 확인).
  - 하단 버튼: [기존 openfortivpn 설정 불러오기]
    - 대상 파일은 `~/.config/openfortivpn/config`, 없으면 `/opt/homebrew/etc/openfortivpn/openfortivpn/config`이다. 읽기만 한다.
    - 가져오는 값과 저장 위치는 다음과 같다.
      - **host → 서버 주소**
      - **port → 포트**(현재 파일 기준 8443. 파일에 port가 없을 때만 443)
      - username → 사용자명
      - realm → realm
      - password → Keychain
      - trusted-cert(64자 hex, 여러 줄 가능) → config.yaml `vpn.trusted_certs`(화면에는 보이지 않음)
    - openfortivpn은 알 수 없는 키를 만나면 그 뒤를 읽지 않는다. 우리 리더는 그와 달리 파일 끝까지 알려진 키를 모두 읽는다. `insecure-cert` 같은 알 수 없는 키는 무시하고 안내만 한다.
    - **첫 실행 자동 불러오기**: config.yaml이 없고 위 파일이 있으면, 버튼을 누르지 않아도 첫 실행 때 같은 방식으로 모든 값을 채운다. 그다음 계정 탭을 열어 결과를 보여준다.
    - 원본 파일은 수정하지 않는다. "평문 비밀번호가 들어 있음" 경고와 [Finder에서 보기]를 보여준다.
  - 안내 문구: "Gmail 필터 `subject:AuthCode` → 스팸으로 보내지 않음"을 권장한다.
  - 인증서 관련 항목은 없다(자동 신뢰).
- **탭 ② 연결 관리**
  - **상태**: 현재 상태, 연결된 시각과 IP, 다음 선제 재연결 시각, 버튼 [연결]/[연결 해제]/[지금 재연결].
  - **옵션**: 앱 시작 시 자동 연결, 로그인 시 실행(SMAppService 상태를 직접 읽고 씀), 세션 최대시간(분), 선제 재연결 켜기/끄기 + 여유분(분), 잠자기 후 재연결 켜기/끄기 + 최소 잠자기 시간(분).
  - **Helper**: 상태(미등록 / 승인 필요 / 실행 중 / 버전 불일치), [시스템 설정 열기](일반 > 로그인 항목 및 확장 프로그램 > 앱 백그라운드 활동), openfortivpn 경로와 버전(1.24.x가 아니면 경고).
  - **설정 파일**: config.yaml 경로, [Finder에서 보기], 파싱 오류가 있으면 줄/열과 함께 표시. 파일에서 계정 정보가 바뀌었으면 "다음 연결부터 적용" 배너를 띄운다.
- **탭 ③ 로그**
  - `NSViewRepresentable`(NSScrollView + NSTextView, TextKit 1, 편집 불가·선택 가능, SF Mono 11pt, ⌘F 찾기 바)로 그린다.
  - 데이터는 `LogStore`(링버퍼 5,000줄 / 2MB)에서 seq 순으로 증분 추가한다.
  - 화면 규칙
    - 원본 줄을 그대로 보여준다.
    - 색은 스트림이 아니라 접두어로 정한다: `ERROR:` 빨강, `WARN:` 주황.
    - stderr 줄은 중립색에 작은 `err` 표시만 붙인다(대부분 정상적인 pppd 메시지이기 때문).
    - 헬퍼 메타 줄은 흐린 회색에 `▸`를 붙이고, 시도 구분선 역할을 한다.
  - 툴바: [자동 스크롤] 토글(맨 아래에 있을 때만 따라감), [지우기](화면만), [모두 복사], [Finder에서 보기].
  - 파일 기록: `LogFileWriter` actor가 `~/Library/Logs/FortiVPNManager/openfortivpn.log`(디렉토리 0700, 파일 0600, 2MB × 5개 회전)에 쓴다. 각 줄은 `시각 [out|err|▸] 원문` 형식이다. 사내 IP나 라우트 정보가 남는다(비밀값은 없다).
- **첫 실행 순서**: 순서는 다음과 같다. 자격증명이 비어 있으면 설정창을 자동으로 연다.
  1. 기존 openfortivpn 설정 자동 불러오기(있을 때)
  2. 헬퍼 등록과 승인 안내
  3. 계정 탭: 불러온 값 확인, Gmail 주소와 앱 비밀번호 입력, 테스트
  4. 첫 연결
- `/Applications` 사본만 `SMAppService.mainApp`/`.daemon`을 register한다(bundleURL로 확인).

### 5.7 저장소
| 데이터 | 위치 | 비고 |
|---|---|---|
| 접속 정보(host/port/username/realm), Gmail 주소, 연결 옵션, **서버 인증서 digest(`vpn.trusted_certs`, 앱이 자동 관리)** | `~/.config/FortiVPNManager/config.yaml` | 디렉토리 0700, 파일 0600. 사람이 편집할 수 있다. digest는 비밀값이 아니다 |
| VPN 비밀번호, Gmail 앱 비밀번호 | 로그인 Keychain (`kSecClassGenericPassword`) | VPN은 service `com.qoor.FortiVPNManager.vpn` / account `username@host`, Gmail은 `.gmail` / `gmail_address`. 계정이 바뀌었는데 항목이 없으면 `needsSetup` → 예전 비밀번호로 시도하지 않아 잠금을 막는다 |
| 로그인 시 실행 | SMAppService 상태 | macOS가 소유하는 상태다(사용자가 시스템 설정에서 끄면 YAML 값과 어긋나므로 YAML에 두지 않는다) |
| 실패 카운터, 최근 시도 기록, 최초 로그인 항목 등록 여부 | `~/Library/Application Support/FortiVPNManager/state.json` (0600) | 앱이 관리하는 기기별 일시 상태다. dotfiles(yadm) 동기화 대상에서 분리한다. 카운터는 `username@host` 단위로 묶는다 |
| 로그 | `~/Library/Logs/FortiVPNManager/` | 설정이 아니다 |
| 창 위치 등 UI 상태 | UserDefaults | |

- **config.yaml 스키마**: 앱이 쓰는 고정 템플릿이며 주석으로 설명한다. 헤더에는 다음을 적는다.
  - 비밀번호는 Keychain에 있다.
  - 설정창에서 저장하면 직접 단 주석과 알 수 없는 키가 사라진다.
  - 숫자 앞에 0을 붙이지 않는다.
  - 공개 저장소에 올리지 않는다.
  ```yaml
  schema_version: 1
  vpn:
    host: ""
    # 기존 openfortivpn 설정이 있으면 첫 실행 때 그 값(예: 8443)으로 채워진다. 443은 파일에 값이 없을 때의 기본값.
    port: 443
    username: ""
    realm: ""
    # 서버 인증서 SHA-256 digest. 앱이 첫 연결 때 자동으로 채우고 관리한다. 직접 수정할 필요 없음.
    # 인증서가 바뀌면 앱이 자동으로 새 값을 앞에 추가한다(최대 4개).
    trusted_certs: []
  otp:
    gmail_address: ""
  connection:
    auto_connect_on_launch: true
    session_max_minutes: 180
    proactive_reconnect: { enabled: true, margin_minutes: 5 }
    reconnect_after_sleep: { enabled: false, min_sleep_minutes: 30 }
  ```
- **로드와 검증** (`FortiConfig.ConfigCodec`, 순수 함수, Yams 6.2.x)
  1. 파일이 64KiB 이하이고 UTF-8인지 확인한다.
  2. `Yams.compose`로 읽는다. 줄/열 단위 오류를 낸다.
  3. 앵커, 별칭, `<<` 병합 키는 거부한다.
  4. 정수는 원문 scalar가 `^(0|[1-9][0-9]*)$`일 때만 받는다. YAML 1.1의 `0443`(8진수), `2:30`(60진수), `0x1BB`(16진수)를 막기 위해서다.
  5. 따옴표로 감싼 숫자나 bool에는 전용 메시지를 낸다.
  6. 범위를 검증하고, 빠진 키는 기본값으로 병합한다. `trusted_certs`에서 `^[0-9a-f]{64}$`에 맞지 않는 항목은 오류로 띄우지 않고 무시한다. 대문자 hex는 소문자로 정규화한다.
  7. `schema_version`을 확인한다.
  - 실패하면 마지막 정상 설정(last-known-good)을 유지하고 오류를 표시한다.
  - YAMLEncoder/Decoder는 매번 새로 만든다(Sendable이 아님).
- **쓰기**
  - realpath를 해석한다(심볼릭 링크 보존). 하드링크(nlink>1)이면 경고한다.
  - 같은 디렉토리에 mkstemp(0600) → write → fsync → rename 순으로 쓴다.
  - 파일 권한이 0600이 아니면 경고하고, 다음 쓰기 때 0600으로 고친다.
  - 앱이 파일을 쓰는 경우는 두 가지다. 설정창에서 저장할 때와 새 서버 digest를 저장할 때(평소에는 일어나지 않음)다. 두 경우 모두 디스크의 최신 내용을 읽어 해당 값만 바꾼 뒤 템플릿으로 다시 쓴다. 그래서 그 사이 사용자가 직접 바꾼 값은 유지된다.
- **감시**
  - 2초마다 stat(inode, mtime, size, realpath)을 확인하고, 앱이 활성화될 때도 확인한다.
  - 바이트가 실제로 바뀌었을 때만 다시 읽는다. 앱이 직접 쓴 변경은 무시된다.
  - 변경은 다음 연결부터 적용한다. 단 `session_max`와 `margin`은 즉시 다시 계산한다(최소 현재 시각 + 1분).
- **Swift 6**: `FortiConfig`는 nonisolated이고 앱은 MainActor다. stat 폴링은 MainActor Task에서 돈다.

## 6. 동작 규칙

### 6.1 연결 시퀀스
1. Readiness를 확인한다.
2. `getVersion`을 호출한다.
3. GmailOTPSession을 시작한다(로그인, 스냅샷, 잔여 메일 정리, IDLE).
4. `connect(attemptID, trustedCerts = config.vpn.trusted_certs)`를 호출한다.
5. 비밀번호를 주입한다.
6. `otpPrompted`가 오면 코드를 넣고 `submitOTP` → 메일을 휴지통으로 옮기고 LOGOUT한다.
7. `tunnelUp`이 오면 `connected` 상태로 바꾸고, underlay를 기록하고, 선제 재연결 타이머를 건다.

### 6.2 종료 분류와 재시도 (리듀서가 ExitFacts로 판단, 위에서 아래 순서로)
| 상황 | 동작 |
|---|---|
| `certRejectedDigests`가 있고 `otpSubmitted` 이전 | digest를 config.yaml `vpn.trusted_certs`에 저장하고 즉시 다시 시도한다(체인당 1회). 카운트하지 않는다. 또 거부되면 네트워크 오류로 본다 |
| 터널이 올라온 상태였고, 헬퍼가 끊은 게 아니며, 2분 이상 유지됨 | 서버 세션 종료로 보고 5초 뒤 재연결한다. 카운터를 초기화한다 |
| 선제 재연결 타이머 | 정상 종료 후 재연결한다 |
| `connectedToGateway` 이전 실패 | 네트워크 오류. 경로가 살아날 때까지 기다린 뒤 10초부터 최대 5분까지 백오프한다. 카운트하지 않는다 |
| `connectedToGateway` 이후, OTP 프롬프트 이전의 인증 실패 | 경로가 바뀌었거나 깨어난 지 30초 이내면 네트워크 오류로 본다. 그 외에는 인증 실패로 카운트한다 |
| OTP 미수신(50초), OTP 거부, 인증 후 터널 실패, 2분 미만 유지, `otpSubmitted` 이후 인증서 거부 | 인증 실패로 카운트한다(인증서 거부면 핀도 추가) |
| 인증 실패 카운트 1 | 90초 뒤 1회 재시도한다 |
| 인증 실패 카운트 2 | `failed(사유)` |
| IMAP 인증 실패 | openfortivpn을 실행하지 않고 `failed("Gmail 앱 비밀번호")` |
| IMAP 네트워크 오류 | 네트워크 백오프 |
| 보조 상한: 최근 60분 동안 인증 시도 12회 초과 | `failed("재연결 과다")` |

- `failed`는 메뉴나 연결 관리 탭에서 `연결`/`지금 재연결`을 눌러야만 풀리며, 이때 카운터도 초기화된다.
- 카운터와 기록은 state.json에 저장되므로 앱을 재실행해도 유지된다.

### 6.3 잠자기와 깨어남
- **연결 중이거나 재시도를 기다리는 중에 잠자기**: 시도를 중단(SIGTERM)하고, IMAP을 닫고, 타이머를 취소한다. 깨어난 뒤 경로가 살아나면 새로 시도한다.
- **연결된 상태에서 깨어남**: 경로와 Router가 잡힐 때까지 최대 30초 기다린 뒤 아래를 순서대로 판단한다.
  1. `plannedReconnectAt`이 지났으면 선제 재연결한다.
  2. 프로세스나 ppp 인터페이스가 없으면 재연결한다.
  3. underlay가 바뀌었으면 재연결한다.
  4. `reconnect_after_sleep` 조건을 만족하면 재연결한다.
  5. 그 외에는 연결을 유지한다.
  - 30초 안에 경로가 잡히지 않으면 판단을 보류하고 연결을 유지한다.

## 7. 프로젝트 구조와 빌드
```
FortiVPNManager/
  project.yml  Makefile  .gitignore(*.xcodeproj, Config/Local.xcconfig, .build/)
  Config/Base.xcconfig        # #include? "Local.xcconfig" (DEVELOPMENT_TEAM; make가 템플릿에서 생성)
  App/                        # @main + AppDelegate adaptor, MenuBarContent, SettingsWindowController,
                              # AccountTab, ConnectionTab, LogTab(LogTextView), ConnectionController, HelperClient,
                              # HelperInstaller, GmailOTPSession, SystemMonitor, ConfigStore(감시), KeychainStore,
                              # StateStore, LogStore, LogFileWriter
  Helper/                     # main.swift, XPCService, BinaryLocator, RouteRepair
  Support/LaunchDaemons/com.qoor.FortiVPNManager.Helper.plist   # Label/MachServices/BundleProgram 리터럴
  Packages/FortiCore/         # Package.swift: Yams 6.2.x 의존(FortiConfig만)
    Sources/FortiShared/      # XPC 프로토콜, DTO(HelperEvent, ExitFacts, OutputLine), BuildID
    Sources/FortiCore/        # ConnectionReducer, RetryPolicy, Readiness, OpenFortiVPNOutputParser(인증서 digest 포함),
                              # PromptDetector, LineAssembler, ConfigRenderer, OTPMatcher, IMAPResponseParser,
                              # OpenFortiVPNConfigReader
    Sources/FortiConfig/      # AppConfig 모델, ConfigCodec(Yams), 템플릿 렌더러, 마이그레이션
    Sources/FortiHelperCore/  # ProcessRunner(posix_spawn), TunnelSupervisor, Watchdog (Yams 비의존)
    Tests/…                   # Swift Testing + Fixtures(실제 로그, 메일 헤더, 가짜 openfortivpn 스크립트, YAML 샘플)
```
- **project.yml**
  - 배포 타깃 macOS 15.0, `SWIFT_VERSION 6`.
  - 앱은 기본 격리를 MainActor로, 헬퍼와 패키지는 nonisolated로 둔다.
  - Hardened Runtime을 켜고, Sandbox는 끄고, entitlement는 비운다.
- **헬퍼 타깃**
  - `PRODUCT_BUNDLE_IDENTIFIER`, `GENERATE_INFOPLIST_FILE`, `CREATE_INFOPLIST_SECTION_IN_BINARY: YES`를 설정한다.
  - `OTHER_CODE_SIGN_FLAGS: --identifier $(PRODUCT_BUNDLE_IDENTIFIER)`로 서명 identifier를 고정한다.
  - `ENABLE_DEBUG_DYLIB: NO`.
- **앱에 헬퍼 넣기**: 의존성은 `embed: true`, `link: false`, `codeSign: true`, `copy.destination: executables`로 둔다. plist는 copyFiles(`wrapper`, `Contents/Library/LaunchDaemons`)로 복사한다.
- **Makefile 타깃**
  - `gen`
  - `build`(`-derivedDataPath .build/xcode`)
  - `test`(`swift test --package-path Packages/FortiCore`)
  - `verify`
    - `codesign -dv`(Identifier/TeamIdentifier), `-d -r-`
    - 요건 문자열로 서로를 `codesign --verify -R`해 본다.
    - provisionprofile이 없는지 확인한다.
    - 헬퍼의 `otool -L`에 @rpath가 없는지 확인한다.
  - `install`: 앱을 정상 종료 → 번들 삭제 → ditto → open. 옛 Helper는 SelfUpdateWatcher로 스스로 교체된다(sudo 불필요).
  - `logs`
  - `btm`

## 8. 보안 요약과 감수하는 위험
- 비밀값은 Keychain에 저장하고, 헬퍼에는 XPC를 통해 메모리로만 전달한다. argv, 설정 파일, 환경변수, 로그(원본 포함)에 남지 않는다. 원본 로그가 안전한 근거는 기본 verbosity와 파이프 사용이다.
- XPC는 양쪽 모두 Team ID + identifier 요건을 건다. 헬퍼는 검증된 필드만 받고 config.yaml을 읽지 않는다. 환경 변수는 PATH만 넘긴다.
- **감수하는 위험** (개인 Mac, 사용자 결정)
  1. **서버 인증서 무조건 자동 신뢰**: 공용 Wi-Fi 등에서 능동적인 중간자 공격자가 가짜 인증서를 제시하면 자동으로 `trusted_certs`에 추가된다. 그 결과 VPN 비밀번호와 실시간 OTP가 넘어갈 수 있다. 앱이 OTP를 자동으로 제출하므로 사람이 눈치챌 틈이 없다. 가짜 게이트웨이가 root pppd에 라우트와 DNS를 주입할 수도 있다.
  2. **같은 사용자 권한의 코드 ⇒ root**: Homebrew 바이너리와 dylib을 변조하거나, Personal Team 키로 앱을 사칭하거나, `/Applications`의 앱을 바꿔치기하는 경로가 있다.
  3. **config.yaml 변조**: 같은 사용자 권한으로 `vpn.host`를 바꾸면 1번과 결합해 자격증명이 다른 서버로 간다. Keychain 키가 `username@host`라서 host가 바뀌면 비밀번호를 다시 확인받아야 하므로, 이 경로는 부분적으로 완화된다.
- 강화 옵션(TOFU + 변경 시 확인, root 소유 바이너리 사본, 게이트웨이 핀 고정과 관리자 인증)은 추후 과제다.

## 9. 구현 단계
- **Phase 0. 환경과 SMAppService 스파이크** — ✅ 2026-10-15 macOS 27.0.1에서 성공: Personal Team 서명으로 register → 승인 → XPC 응답 확인. launchd LWCR이 cdhash가 아닌 Team ID + signing identifier로 고정됨 → 재빌드 시 재승인 불필요. 레거시 LaunchDaemon 대체안은 불필요. 추가 확인: 재설치 후 Keychain 접근 프롬프트 없음. **재설치 시 실행 중인 옛 Helper는 디스크 바이너리가 바뀌어 앱의 서명 검증을 통과하지 못한다** → Helper의 `SelfUpdateWatcher`가 자기 실행 파일 교체를 감지(5초 폴링 + 새 연결 시)해 유휴 상태면 스스로 종료하고, 앱은 연결 실패 시 1.5초 후 1회 재시도한다. 따라서 `make install`에 sudo가 필요 없다. 또한 앱(MainActor 기본 격리)에서 XPC 오류/무효화 콜백은 반드시 `@Sendable`로 선언해야 한다(미선언 시 런타임 격리 검사로 크래시). 서명 인증서가 유효하려면 로그인 키체인에 Apple WWDR G3 중간 인증서가 필요했다.
  1. `sudo xcode-select -s /Applications/Xcode.app`, 라이선스 동의, `-runFirstLaunch`.
  2. Xcode Accounts에 Apple ID를 등록하고 Apple Development 인증서를 만든 뒤 Team ID를 확인한다.
  3. `brew install xcodegen`.
  4. 최소 앱과 데몬, 확정된 번들 ID로 다음을 확인한다: register, 승인, XPC ping, `/Applications` 교체 후 동작, 재빌드 후 Keychain 프롬프트가 뜨지 않는지, 재부팅 후 동작, AhnLab 간섭 여부.
  - 실패하면 레거시 LaunchDaemon 설치로 바꾼다.
- **Phase 0b. 기능 스파이크** (사용자 참여, 반나절)
  - scratchpad에서 일회용 `python3 -I` imaplib 스크립트로 IMAP 로그인, SPECIAL-USE, IDLE, `UID MOVE`를 확인한다.
  - 사용자가 수동으로 약 5회 접속하는 동안 다음을 측정한다.
    - OTP 메일 도착 지연
    - 실제 Subject/Date
    - OTP 프롬프트 경로(tokeninfo인지 HTML 폼인지)
  - 다음 로그를 확보한다.
    - 타임스탬프가 붙은 **자연 종료(~3h) 로그**: 세션 최대시간 확정과 파서 픽스처용. stdout/stderr를 따로 기록한다.
    - 10분 잠자기 테스트 결과
    - 인증서 거부 출력 1회: `trusted-cert` 없이 실행하면 얻을 수 있다.
  - Go/No-go 기준: IMAP 로그인 성공, OTP 도착 p95 30초 미만.
- **Phase 1. 패키지**
  - FortiCore: 리듀서, 재시도 정책, Readiness, 파서, PromptDetector, LineAssembler, 렌더러, OTPMatcher, IMAP 파서, ConfigReader.
  - FortiConfig: Codec, 템플릿.
  - FortiHelperCore: ProcessRunner, TunnelSupervisor, 워치독.
  - 모두 테스트를 함께 작성한다.
- **Phase 2. Helper**: XPCService, output 배치 전송, BinaryLocator, RouteRepair, 고아 정리, 시그널 처리.
- **Phase 3. App**: ConfigStore, StateStore, KeychainStore, 설정창 3개 탭, 메뉴바, HelperClient와 Installer, GmailOTPSession, ConnectionController, SystemMonitor, LogStore와 LogFileWriter, 로그인 항목.
- **Phase 4. 통합 검증**: 10절 전체를 수행하고, Release 빌드를 `/Applications`에 설치한다.

## 10. 검증
- **`make test`**
  - **리듀서**
    - 서버 종료 후 재연결
    - 인증 실패 1회 → 90초 → 다시 실패 → failed
    - **인증서 거부 → `trusted_certs` 저장 이펙트 → 즉시 재시도(카운트 없음) → 다시 거부되면 네트워크 오류**
    - digest가 저장된 뒤의 연결 요청에 그 값이 실려 나가는지(평소 1회 실행 보장)
    - host/port 변경 시 `trusted_certs` 초기화, 최대 4개 유지
    - `otpSubmitted` 이후 인증서 거부 → 카운트
    - 빠른 끊김 루프, 수동 조작 시 카운터 초기화, 보조 상한
    - 연결 중 잠자기
    - 오래된 attemptID 무시
    - `needsSetup` → 자동 연결
    - username/host 변경 → `needsSetup`
  - **파서/PromptDetector/LineAssembler**
    - 실제 1.24.1 로그
    - 줄바꿈 없는 프롬프트와, 비밀번호 직후 개행이 다음 줄에 붙는 경우
    - 두 OTP 프롬프트 경로
    - 인증서 거부 블록(중복 출력, 인증 이후 단계에서 출현)
    - 250ms 부분 줄 flush, 단독 LF, 16KB 상한, 잘못된 UTF-8
    - pppd 타임스탬프 줄이 조각으로 들어오는 경우
  - **렌더러**: 여러 trusted-cert 줄, 개행/제어문자/길이/잘못된 digest 거부.
  - **OpenFortiVPNConfigReader**: 사용자 실제 파일 형식(마스킹한 픽스처)에서 다음을 확인한다.
    - host, `port = 8443`, username, password, trusted-cert를 모두 추출한다.
    - 알 수 없는 키(`insecure-cert`)가 앞이나 중간에 있어도 이후 키를 읽는다.
    - port가 없으면 443으로 채운다.
    - 공백과 주석 줄을 처리한다.
  - **ConfigCodec**
    - 기본값 병합
    - `0443`/`2:30`/`0x1BB` 거부
    - 따옴표로 감싼 숫자 메시지
    - 앵커/별칭/`<<` 거부
    - 64KiB 상한
    - 줄/열 오류 위치
    - 템플릿 왕복
    - schema_version 처리
  - **OTPMatcher와 IMAP 파서**: 기존 항목 그대로.
  - **TunnelSupervisor**: 가짜 스크립트로 확인한다.
    - stdin 주입
    - 워치독
    - 백그라운드 큐에서 spawn하고 부모가 SIGTERM을 무시하는 상태에서도 자식이 1초 안에 종료되는지
    - EPIPE
    - output 배치 순서와 `.exited` 전 flush
- **수동 E2E**
  1. 첫 실행부터 연결까지
     - config.yaml이 없는 상태로 첫 실행하면 기존 openfortivpn 설정을 자동으로 불러온다. host, **port 8443**, username, 비밀번호(Keychain), `trusted_certs`가 채워져야 한다.
     - 이어서 헬퍼를 승인하고 Gmail 정보를 입력한다.
     - 첫 연결은 openfortivpn **1회 실행**으로 성공해야 한다(가져온 digest 사용).
     - OTP가 자동으로 입력되고, "연결됨"이 표시되고, 로그 탭에 원본 로그가 보여야 한다.
  2. 인증서 digest 자동 저장을 확인한다.
     - config.yaml의 `trusted_certs`를 비운 뒤 연결: 로그 탭에 거부 블록 1회 → 즉시 재실행 → 연결. `trusted_certs`에 digest가 저장된다.
     - **그다음 연결부터는 거부 블록 없이 openfortivpn이 1회만 실행된다**(로그 탭의 시도 구분선으로 확인).
  3. `sudo kill -TERM <pppd pid>`: 재연결된다. 실제 서버 종료와 출력이 다를 수 있다는 점을 감안한다.
  4. `sudo kill -9 <openfortivpn pid>`: RouteRepair 후 인터넷이 정상이고 재연결된다.
  5. 세션 최대시간을 10분으로 바꿨을 때 선제 재연결이 동작한다.
  6. 잠자기 3종(짧은 잠자기, 오래 잠자기, 연결 중 잠자기)과 Wi-Fi 변경.
  7. 비밀번호 오류 → 1회 재시도 → failed.
  8. Gmail 앱 비밀번호 오류 → openfortivpn 미실행.
  9. OTP 메일 차단 → failed.
  10. config.yaml 직접 편집
      - 옵션 변경이 반영되는지
      - `port: 0443`이 거부되는지
      - 깨진 YAML에서 last-known-good이 유지되고 오류가 표시되는지
      - username 변경 시 `needsSetup`이 되는지
      - 심볼릭 링크로 된 config.yaml을 UI에서 저장해도 링크가 유지되는지
  11. 앱 강제 종료 → 15초 안에 프로세스 정리와 라우트 복원.
  12. Quit → 정상 종료.
  13. `pmset -g assertions`에 수면을 막는 assertion이 없다.
- **수용 기준**
  - 선제 재연결을 끈 상태로 자연 종료를 2회, 켠 상태로 1회 겪는다.
  - 그동안 수동 조작이 없어야 하고, 끊긴 시간은 각각 90초 이하여야 한다.
  - 휴지통으로 옮긴 OTP 메일 수가 인증 시도 횟수와 같아야 한다.
  - 고아 프로세스가 없어야 한다.
  - 로그 파일에 자연 종료 구간이 남아 있어야 한다.
- **보안 확인**
  - `ps`에 비밀값이 보이지 않는다.
  - 렌더링된 설정 파일은 0600이고 종료 후 삭제된다.
  - 로그 파일과 `log show`에서 그 회차의 OTP 코드나 비밀번호를 grep해도 나오지 않는다.
  - config.yaml과 state.json은 0600이다.
  - `make verify`가 통과한다.

## 11. 리스크와 추후 과제
- FortiOS 7.6.3 이상에서는 SSL VPN 터널 모드가 제거됐다. 게이트웨이를 업그레이드하면 openfortivpn을 쓸 수 없다. 대안 후보는 openconnect `--protocol=fortinet`이다.
- 잠자기 중 PPP 커널 확장 패닉 보고가 있다(#1278). 실사용에서 관찰한다.
- 헬스체크가 프로세스와 인터페이스로 한정돼 있어, 서버 idle-timeout으로 죽은 연결은 늦게 감지될 수 있다. Phase 0b 측정 결과로 `reconnect_after_sleep` 기본값을 정한다.
- 설정창에서 저장하면 config.yaml에 직접 단 주석이 사라진다(Yams가 주석을 보존하지 않음). 헤더에 이 점을 명시한다.
- Personal Team 인증서는 1년마다 만료된다. openfortivpn을 업그레이드하면 출력 문자열이 바뀔 수 있으므로 버전 경고와 픽스처 회귀 테스트로 대응한다.
- 로그 탭 추가로 Phase 1과 Phase 3의 작업량이 늘었다(LineAssembler, output XPC, LogStore/FileWriter, NSTextView 뷰).
- 추후 과제: 지인 배포(각자 빌드 / 레거시 설치 경로 / 유료 계정 공증), 알림, 인증서 TOFU 강화, split DNS, 다중 프로필, adopt, root 소유 바이너리 사본, Gmail OAuth, openconnect 백엔드.
