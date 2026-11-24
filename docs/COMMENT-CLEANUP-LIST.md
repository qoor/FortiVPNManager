# core 주석 정리 대상 (임시)

- 기준: 브랜치 `refactor/core-decision-action`, commit `8355cdd`
  - 줄 번호: 이후 변경으로 이동 가능, 인용문으로 위치 확인
- 대상: `components/core/src` 주석, 시험 코드 제외
- 진행 시점: 모든 정리 작업 완료 후 별도 작업
- 작업 위치: 별도 브랜치

## 판정 기준
- 주석 미작성 원칙
- 문서 주석 허용 대상: 역할, 입력값, 결과값, 오류·panic 조건, 사용 예
- 문서 주석 기재 금지: 코드를 그대로 옮긴 설명, 바뀔 수 있는 결정·판단
- 코드로 안 보이는 결정과 이유: `decisions.md`
- 외부 제약·실측 사실: `constraints.md`

## 분류

| 코드 | 처리 |
|---|---|
| T | 역할 문장만 유지, 나머지 제거 |
| D | 결정·이유를 `decisions.md`로 이동 |
| C | 외부 제약을 `constraints.md`로 이동 |
| X | 삭제 |
| S | 코드와 불일치, 정정 또는 삭제 |

- 복합 표기: `T+D`, `T+C`
  - 역할 문장 유지와 이동 함께 적용

## 결과 요약

| 처리 | 개수 |
|---|---|
| 유지 | 639 |
| T | 73 |
| X | 25 |
| D | 12 |
| S | 10 |
| C | 5 |

## 보류: 근거 확인 불가 제약
- 처리 방침: 근거 확인 전 주석 유지
  - 진행 위치: todolist "제품 판단 규칙의 근거 전수 확인"

| 위치 | 인용 | 확인할 사실 |
|---|---|---|
| `gmail_api.rs:103` | "Gmail's are URL-safe already" | Gmail ID의 URL 안전 문자 한정 |
| `oauth.rs:303` | "about 75 and 35 characters" | client ID·secret 길이 |
| `openfortivpn_settings.rs:31` | "built with /usr/sbin/pppd" | macOS 빌드의 pppd 키 미지원 |
| `otp_mail.rs:266` | "hexadecimal and grow over time" | Gmail ID의 시간순 증가 |
| `state.rs:12` | "a few small values" | state.json 상한 1MiB의 근거 |
| `config/tree.rs:14` | "the parser itself stops flow nesting sooner" | saphyr flow 중첩 상한 |
| `connection/classify.rs:14` | "the facts may lag behind it" | Helper 사실의 milestone 지연 |
| `connection/mod.rs:395` | "in case the unlock went unreported" | OS의 잠금 해제 미통보 |
| `connection/mod.rs:433` | "DNS may answer with another address" | 터널 안 DNS의 다른 주소 응답 |
| `connection/policy.rs:14` | "FortiGate blocks the IP" | 인증 2회 실패 후 IP 차단 |
| `machine/presence/decision.rs:27` | "in case the unlock went unreported" | `connection/mod.rs:395`와 같음 |
| `machine/helper/decision.rs:107` | "The connection stays inside the Mac" | 앱↔Helper 연결의 Helper 종료 시 한정 끊김 |
| `machine/timers/action.rs:55` | "a timer may not survive the sleep" | 잠자기 뒤 타이머 유실 |
| `oauth/loopback.rs:5` | "well under 2 KiB" | 8KiB 상한의 근거 |
| `otp_mail/header.rs:29` | "raw 8-bit UTF-8 subjects occur" | 실제 메일의 raw UTF-8 제목 |

## 연결 판정 (`connection/`, `machine/`)

| 위치 | 처리 | 인용 | 비고 |
|---|---|---|---|
| `connection/certs.rs:22` | T+D | "they belong to one gateway" | 서버 주소·포트 변경 시 신뢰 인증서 삭제 |
| `connection/classify.rs:46` | X | "The app stopped the run because the Gmail session failed" | 코드 설명 |
| `connection/classify.rs:74` | S | "The app's own deadline stop raced the tunnel" | 앱 중지가 없는 분기 위에 위치 |
| `connection/mod.rs:50` | T | "so the manager stays deterministic" | 사유 제거 |
| `connection/mod.rs:335` | T+D | "the values belong to the old gateway" | `certs.rs:22`와 같은 결정 |
| `connection/mod.rs:458` | T | "a quiet evening can be told from a failure" | 사유 제거 |
| `connection/policy.rs:49` | T | "never sooner than a minute from now" | 상수 값 반복 |
| `connection/readiness.rs:9` | T | "so the OS asks the user for nothing" | 사유 제거 |
| `connection/readiness.rs:58` | T | "every two seconds" | 상수 값 반복, 사유 제거 |
| `connection/schedule.rs:72` | T | "a week and a day on either side" | 정당화 제거 |
| `connection/schedule.rs:92` | C | "A daylight saving change can move a start" | 일광 절약 시간 특이사항 |
| `connection/system.rs:9` | S | "this Mac's address on it" | 두 OS 공용 코드의 Mac 한정 표현 |
| `connection/system.rs:61` | T | "every second; after that, every two seconds" | 상수 값 반복 |
| `connection/view.rs:143` | S+D | "Waiting for an away user is not a reconnect" | 한 줄 위 분기에 위치, 부재 대기의 미연결 표시 |
| `connection/view.rs:287` | T+D | "account edits never touch a running tunnel" | 계정 수정의 다음 연결 적용 |
| `machine/mod.rs:4` | T | "so no transition can forget" | 사유 제거 |
| `machine/mod.rs:182` | T+C | "the gateway sends the code email once it accepts them" | 자격 증명 수락 후 OTP 메일 발송 |
| `machine/mod.rs:294` | T | "bound by the schedule itself" | 제외 사유 제거 |
| `machine/attempts/decision.rs:450` | D | "A run started before a server address or port change" | 인증서 학습의 현재 게이트웨이 한정 |
| `machine/finish/decision.rs:54` | D | "A pending stale-code search keeps its polling" | 미사용 코드 탐색 중 활동 유지 |
| `machine/helper/decision.rs:148` | X | "The certificate check passed" | 필드 문서와 중복 |
| `machine/helper/decision.rs:180` | X | "Still open only when the gateway asked for no OTP" | 상태 설명 |
| `machine/helper/decision.rs:205` | D | "No code email follows a login the gateway rejected" | 거부된 로그인 뒤 미사용 코드 탐색 생략 |
| `machine/helper/decision.rs:251` | X | "An attempt already finished with" | 코드 설명 |
| `machine/helper/decision.rs:341` | T+D | "A stop decided before the run's Connect left" | 시작 전 중지된 실행 미전송 |
| `machine/helper/decision.rs:362` | T+D | "The exception routes are in place exactly then" | 예외 route 적용 시점, 사용자 결정 |
| `machine/ledger/action.rs:12` | T | "A tunnel up proves the credentials work" | 역할 문장 없음 |
| `machine/otp/action.rs:73` | X | "Nothing looks for the code email any more" | 코드 설명 |
| `machine/otp/action.rs:126` | X | "The session moves the used emails to the trash" | Effect 문서와 중복 |
| `machine/otp/decision.rs:77` | T+D+S | "the gateway may have sent a code email" | 미사용 코드 메일 탐색, 사용자 결정, Mac 표현 |
| `machine/otp/decision.rs:246` | D | "The spam check tells at once" | OTP 기한 시 탐색 생략 |
| `machine/outcomes/decision.rs:45` | D | "The chain's one immediate retry belongs to a gateway" | 인증서 즉시 재시도의 게이트웨이별 1회 |
| `machine/outcomes/decision.rs:62` | T+D | "the Helper ends itself on internal faults" | 강제 종료 후 즉시 재연결 1회 |
| `machine/outcomes/decision.rs:112` | D | "An authentication retry is bound by the ledger's cooldown" | 경로 대기 중 인증 재시도 지연 0 |
| `machine/presence/decision.rs:22` | T | "The away time may have run out" | 특이사항 제거 |
| `machine/presence/decision.rs:54` | T | "earlier ones were missed in a sleep" | 사유 제거 |
| `machine/readiness/decision.rs:97` | D | "Applies to the next attempt" | 실패 해제는 사용자 한정 |
| `machine/readiness/decision.rs:139` | T | "every two seconds" | `connection/readiness.rs:58`과 중복 |
| `machine/requests/decision.rs:84` | T+D | "Restarting an attempt under way could waste an OTP" | 진행 중 시도의 지금 재연결 무시 |
| `machine/requests/decision.rs:161` | X | "The next connection learns the new gateway's certificate" | 인증서 규칙 주석과 중복 |
| `machine/system/action.rs:82` | X | "Whether the screen is locked after the wake decides presence" | 다른 주석과 중복 |
| `machine/system/decision.rs:42` | T | "every second; after that, every two seconds" | 상수 값 반복 |
| `machine/system/decision.rs:152` | S | "away while the Mac sleeps" | Mac 한정 표현, 분기 설명 |
| `machine/system/decision.rs:353` | T+D | "a wrong 'gone' would replace a working tunnel" | 터널 IP 미보고 시 터널 존재 판정 |
| `machine/system/decision.rs:393` | D | "The wake rules judge a tunnel that was up across the sleep" | 깨어남 판정 전 올라온 터널 제외 |
| `machine/system/decision.rs:437` | X | "The network did not come back" | 분기 설명 |
| `machine/system/decision.rs:497` | D | "While a wake is being judged" | 깨어남 판정 중 경로 변화 무시 |
| `machine/system/decision.rs:503` | D | "An attempt under way or a stop ignores the path" | 진행 중 시도·중지의 경로 변화 무시 |
| `machine/timers/decision.rs:60` | D | "A fired timer still wanted at the same time stays fired" | 반복 발화 방지, `decisions.md`에 결과만 기록 |

## 설정·상태 파일 (`config/`, `state.rs`)

| 위치 | 처리 | 인용 | 비고 |
|---|---|---|---|
| `config/mod.rs:218` | T | "so a newer app's file stays as it is" | 사유 제거 |
| `config/reader.rs:1` | T | "a newer app's file may break this version's rules" | 사유 제거 |
| `config/reader.rs:75` | X | "Reported where the structure is checked" | 코드로 확인 가능 |
| `config/reader.rs:135` | S | "there are no anchors, aliases or tags" | 키의 tag 허용과 불일치 |
| `config/reader.rs:386` | T+D | "the app manages the list and learns a value again" | 인증서 목록 이상값의 제외 처리 |
| `config/reader.rs:405` | X | "Newest first: the rest are older values" | 상수 문서와 중복 |
| `config/reader.rs:429` | T+D | "it would change what goes around the VPN" | 예외 대상 이상값의 파일 실패 처리 |
| `config/reader.rs:609` | T+D | "the only spelling YAML 1.1 and 1.2 read the same" | YAML 버전 간 같은 해석 표기 한정 |
| `config/template.rs:1` | T+D | "a limit the user accepted" | 쓰기 시 사용자 주석·미지 키 소실 수용, 사용자 결정 |
| `config/template.rs:20` | T | "only the contract's rule produces them" | 사유 제거 |
| `config/text.rs:73` | T+C | "YAML 1.1 reads those as line breaks" | NEL·LS·PS의 YAML 버전별 해석 차이 |
| `config/text.rs:82` | T | "which matters only for an invalid byte" | 판단 제거 |
| `config/tree.rs:1` | T | "so that nesting costs no stack" | 사유 제거 |
| `state.rs:46` | X | "MARK: Wire format" | 구획 표시 |
| `state.rs:48` | D | "Earlier builds also wrote appliedExceptionRoutes" | 이전 빌드 필드의 읽기 무시·쓰기 제외 |
| `state.rs:104` | T | "as in the app's other JSON" | 사유 제거 |
| `state.rs:198` | T | "readable when someone opens the file" | 사유 제거 |

## 메일·인증 (`imap`, `gmail_api`, `otp_mail`, `oauth`)

| 위치 | 처리 | 인용 | 비고 |
|---|---|---|---|
| `gmail_api.rs:80` | T | "which Gmail empties after 30 days" | 코드와 무관한 외부 사실 |
| `gmail_api.rs:162` | T | "so it is safe to log" | 사유 제거 |
| `imap.rs:124` | T+C | "`\All` leaves spam out" | Gmail IMAP `\All`의 스팸 제외 |
| `imap.rs:191` | T+C | "returns the newest older message for n:*" | `UID n:*`의 n 미만 최신 메일 응답(RFC 3501) |
| `imap.rs:203` | T | "After a change the UIDs mean nothing" | 사유 제거 |
| `imap.rs:230` | T+D | "Only refusals that clearly mean 'later'" | 영구·일시 거부 응답 판정 기준 |
| `imap/command.rs:54` | T+C | "answers n:* with the newest message" | `imap.rs:191`과 같은 제약 |
| `imap/command.rs:88` | T+C | "LIST returns 7-bit names" | UTF8=ACCEPT 미사용 시 7비트 이름 |
| `imap/parser.rs:5` | T | "so the grammar's recursion stays bounded" | 사유 제거 |
| `imap/parser.rs:396` | X | "a trailing CONDSTORE (MODSEQ n)" | 코드 설명 |
| `imap/parser.rs:467` | X | "The hierarchy delimiter, a string or NIL" | 코드 설명 |
| `otp_mail.rs:1-17` | T+C+D+S | "`FortiGate`'s OTP email has the subject `AuthCode: NNNNNN`" | 제목 `AuthCode:`·발신자 고정 없음, 미사용 코드 메일 탐색 결정, Mac 표현 |
| `otp_mail.rs:30` | T+C+S | "INTERNALDATE has one-second resolution" | 5초 허용의 근거, Mac 표현 |
| `otp_mail.rs:36` | T+D | "the IMAP `\All` mailbox leaves spam and trash out" | IMAP 범위와 맞춘 제외 |
| `otp_mail.rs:60, 171` | X | "MARK: …" | 구획 표시 |
| `otp_mail.rs:208` | T | "nothing shows they arrived after the session opened" | 사유 제거 |
| `otp_mail.rs:221` | X | "An ID with its arrival; None sorts first" | 타입 설명 |
| `otp_mail/header.rs:41` | X | "unfolding removes only the line break" | 코드 설명 |
| `otp_mail/header.rs:172` | X | "An RFC 2231 language suffix" | 코드 설명 |
| `otp_mail/header.rs:211` | T+C | "CP949, a common name for Korean that the labels lack" | WHATWG 라벨에 CP949 없음 |
| `oauth.rs:32` | T+C | "Google accepts for Desktop app clients on any port" | Desktop client의 loopback 임의 포트 허용 |
| `oauth.rs:78` | T+C | "makes Google return a refresh token" | offline·consent 요청 시 refresh token 발급 |
| `oauth.rs:137` | T+C | "revoking a refresh token revokes its access tokens" | Google revoke 동작 |
| `oauth.rs:143` | T+C | "Google reads an unescaped + as a space" | Google 폼 해석 |
| `oauth.rs:173, 293` | X | "MARK: …" | 구획 표시 |
| `oauth.rs:179` | T | "typically 3599 seconds" | 외부 수치 제거 |
| `oauth.rs:183` | T+C | "Google may grant fewer scopes" | 사용자 해제 시 일부 scope 부여 |
| `oauth.rs:295` | T | "PKCE protects the sign-in" | 사유 제거 |
| `oauth/loopback.rs:7` | C | "Google's authorization codes are about 73 characters" | 2048 상한의 근거 |
| `oauth/loopback.rs:194` | T | "so it is safe to show and log" | 사유 제거 |

## 기타 (`bypass_rules.rs`, `openfortivpn_settings.rs`)

| 위치 | 처리 | 인용 | 비고 |
|---|---|---|---|
| `bypass_rules.rs:18` | S | "The network this Mac is on" | Mac 한정 표현 |
| `bypass_rules.rs:47` | T+D | "server always goes out directly" | VPN 서버 주소의 예외 대상 불가, 사용자 결정 |
| `bypass_rules.rs:56, 145` | X | "MARK: …" | 구획 표시 |
| `bypass_rules.rs:61, 75, 356` | S | "This Mac's address", "this Mac's own network" | Mac 한정 표현 |
| `openfortivpn_settings.rs:4-14` | T+C | "follows openfortivpn 1.24.1's load_config()" | openfortivpn 1.24.1 설정 해석 규칙 |
| `openfortivpn_settings.rs:83` | T+C | "openfortivpn stops reading at the first one" | `4-14`와 같은 제약 |
| `openfortivpn_settings.rs:149` | C | "a refused port leaves the previous one" | openfortivpn의 거부된 포트 처리 |
| `openfortivpn_settings.rs:226` | X | "The leading zero itself reads as an octal digit" | 코드 설명 |
| `openfortivpn_settings.rs:237` | X | "the value saturates" | 코드 설명 |
