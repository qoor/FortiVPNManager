# Rust 사전 리팩토링 계획

상태: 1단계 준비 중 (2026-11-06 시작). 모든 단계가 끝나면 결정과 교훈을 [DESIGN.md](DESIGN.md)의 결정 이력으로 옮기고 이 문서는 지운다.

## 1. 목적

나중에 다른 OS(Linux, Windows)와 backend(openconnect)를 붙일 때 UI와 OS 연동만 새로 만들면 되도록 구조를 바꾼다. 공통 로직은 Rust로 옮기고 구성 요소를 나눈다. 무엇을 하는지는 [SPEC.md](SPEC.md)가 기준이다.

## 2. 원칙

- **사용자가 보는 동작은 바꾸지 않는다.** 승인된 예외는 세 가지다.
  1. config.yaml에 raw NEL·LS·PS가 있으면 오류로 처리한다.
  2. YAML parser를 바꾸면서 일부 오류의 줄·열 위치와 드문 문법의 처리가 조금 달라진다. 값을 다르게 읽는 경우는 없다.
  3. openfortivpn 실행 파일 문제를 종류와 경로로 넘기고, UI가 문장을 만들어 번역한다(2단계).
- **저장소 호환**: config.yaml, state.json(실패 기록 포함), Keychain 항목, 로그 파일(위치, 줄 형식, 회전)을 그대로 쓴다. 1단계 동안 앱 ↔ Helper 메시지는 지금과 바이트 수준으로 같다.
- **진행 방식**: 단계마다 아래 순서로 진행한다. 코드는 background agent가 쓴다.
  1. SPEC.md 기준으로 구현한다.
  2. 자동 테스트를 통과시킨다.
  3. 코드 리뷰를 한다.
  4. 결과를 보고하고 승인을 받는다.
- **기존 코드를 참고하는 범위**: 바꿀 수 없는 외부 사실을 확인할 때만 본다. XPC 메시지 모양, 시간 값과 횟수, openfortivpn 출력 문구와 실행 조건이 여기에 해당한다. 기존 구조와 흐름은 따르지 않는다.

## 3. 구조

### 3.1 디렉토리

구성 요소마다 디렉토리를 하나 둔다. 구성 요소 안에서 OS에 의존하는 부분만 `macos/` 같은 하위 디렉토리로 나눈다.

```
components/
  contract/                 공통 계약 (macos/: 메시지 읽기·쓰기, 앱 ↔ Helper 연결 정의)
  core/                     판단 코어
  runtime/                  런타임 (macOS 앱이 Rust를 시작하는 부분은 crate 안의 macos/)
  backend-adapter/          backend adapter (openfortivpn/, 나중에 openconnect/)
  watchdog/                 watchdog (프로세스 처리: unix/, 나중에 windows/)
  helper-entry/macos/       Helper entry
  ui/macos/                 UI
  os-integration/macos/     OS 연동
  os-integration-helper/macos/  OS 연동 Helper
packaging/macos/            Xcode 프로젝트 정의, 빌드 설정, LaunchDaemon 정보
docs/  scripts/  Cargo.toml  Makefile  README.md
```

- **단계별 정리**: `App/`은 2단계에서 `ui/macos/`와 `os-integration/macos/`로 나누고 나머지는 지운다. `Packages/`는 1단계에서 Helper 전용 부분을, 2단계에서 나머지를 지운다.
- **crate 이름**: `fvm-` 접두사를 붙인다. Rust의 기본 라이브러리 `core`와 이름이 겹치지 않게 하기 위해서다.

### 3.2 의존 방향

- 판단 코어 → 공통 계약
- 런타임 → 판단 코어, backend adapter, 공통 계약
- backend adapter → 공통 계약
- watchdog → 공통 계약
- Helper entry → watchdog, backend adapter, 공통 계약

판단 코어와 watchdog은 VPN client 이름을 모른다. 런타임이 backend adapter를 쓰는 곳은 기존 설정 가져오기, 인증서 값 형식 확인, 계정 값 확인 세 가지뿐이다.

### 3.3 기술 결정

| 항목 | 결정 |
|---|---|
| Swift ↔ Rust | 프로세스 안의 이름 없는 socketpair 위로 4바이트 big-endian 길이와 UTF-8 JSON을 주고받는다. 시작 함수는 런타임과 Helper entry에 하나씩 둔다. Rust가 양쪽 끝에 SIGPIPE 금지를 켠다. Swift는 FileHandle과 Data만 쓴다. 읽는 쪽은 하나이고, 쓰기는 background serial queue에서 하며, main actor 전환은 한 곳에서 한다. UniFFI와 별도 프로세스 IPC는 쓰지 않는다 |
| 메시지 형식 | JSON. Swift 쪽 타입은 Rust가 만든 예시 JSON(fixture)으로 맞는지 테스트한다 |
| UI 상태 | 도메인 구역 단위로 보낸다. 바뀐 구역만 통째로 보내고, 로그는 추가분만 보낸다 |
| macOS 영역 | UI, OS 연동, OS 연동 Helper는 Swift. macOS UI는 앞으로도 Swift로 만든다 |
| unsafe | Rust에 unsafe 블록을 두지 않는다. 외부로 내보내는 시작 함수의 이름 표시만 허용한다 |
| 오류 | panic은 abort한다. Release 빌드에도 줄 번호를 남긴다 |
| YAML | saphyr-parser의 이벤트 API를 쓰고, 엄격 규칙은 직접 확인한다 |
| TLS | rustls에 ring provider 하나와 platform verifier를 쓴다 |
| 빌드 | Rust 2024 edition, Apple Silicon 전용. XcodeGen의 pre-build script가 cargo를 실행한다 |
| 앱 크기 | 줄이는 작업은 모든 단계가 끝난 뒤에 따로 한다 |
| 나중에 | Linux UI는 Rust + QML(cxx-qt). Windows backend는 openconnect, driver는 Wintun 기본 |

## 4. 단계

| 단계 | 상태 | 내용 | 확인 |
|---|---|---|---|
| 0. 기반 검증 | 완료 | Rust 연결, socketpair, Rust 통신, YAML 선택, spike 설치 | 시험 프로그램 전부 통과, 설치 뒤 Helper 재승인 없음 |
| 이동 | 완료 | `project.yml`·`Config/`·`Support/`를 `packaging/macos/`로, `Helper/`를 `components/os-integration-helper/macos/`로 옮긴다. 내용은 바꾸지 않는다 | 빌드, `make verify`, 기존 테스트 |
| 1. Helper 쪽 | 대기 | 공통 계약, backend adapter(openfortivpn), watchdog, Helper entry를 만든다. OS 연동 Helper를 줄인다. SPEC F-VR-01~07, F-SW-03 | Helper 시나리오 Rust 테스트, 지금 앱과 XPC 양방향 fixture, Log 설명 줄 일치, `make verify`, DESIGN §10의 1·2·3·5·6·8·9와 재설치 |
| 2. 앱 쪽 | 대기 | 판단 코어, 런타임, UI, OS 연동, macOS 쪽 공통 계약을 만들고 한 번에 전환한다 | 앱 시나리오와 YAML 사례, 저장소 호환, 구역별 UI 문장 일치, DESIGN §10 1~9 |
| 3. 문서 | 대기 | DESIGN.md를 새 구조로 다시 쓰고(§9 정정 포함), README를 고친다 | |

## 5. 승인된 결정

- **(a)** 아래 정리 항목 11개를 모두 적용한다.
- **(b)** Log 탭 [모두 복사]는 UI가 화면에 가진 줄로 만든다.
- **(c)** 실행 파일 문제는 종류와 경로로 넘기고, UI가 문장을 만들어 번역한다(2단계).
- **보류 조건**: watchdog의 프로세스 생성 조건을 unsafe 없이 지킬 수 없으면, 진행하기 전에 다시 묻는다.

## 6. 정리 항목

1. **IPv4 읽기를 하나로**: 다섯 곳이 조금씩 다른 규칙으로 IPv4를 읽는다. 공통 계약의 규칙 하나로 모은다.
2. **인증서 값 형식 확인을 하나로**: 연결 쪽과 config.yaml 쪽이 같은 규칙을 따로 갖는다. backend adapter의 것만 둔다.
3. **검증된 버전 판단을 UI에서 backend adapter로**: 버전 규칙은 openfortivpn에만 해당하는 처리다. UI에는 경고 문장만 남는다.
4. **실행 파일 문제를 문장 대신 종류와 경로로**: 문장은 UI가 만든다는 규칙에 맞춘다.
5. **테스트용 시계 주입을 옮기지 않음**: 시간 값이 모두 입력이라 짧은 값과 실제 시간으로 테스트할 수 있다.
6. **인증서 값 비우기 규칙을 하나로**: 설정창 저장과 Connection Manager가 같은 규칙을 따로 갖는다. 판단 코어의 것만 둔다.
7. **"다음 연결부터 적용"과 상태 줄 판단을 UI에서 판단 코어로**: UI에 판단을 남기지 않는다는 규칙에 맞춘다.
8. **큰 스택 스레드를 옮기지 않음**: YAML 구조를 재귀 대신 반복으로 만들면 필요 없다.
9. **준비 상태 항목의 이름을 뜻대로**: "Gmail 앱 비밀번호 있음"은 실제로 "Gmail 인증 있음"(앱 비밀번호 또는 Google 계정)이다.
10. **OTP 세션의 시도 기록을 한 곳으로**: 방식 선택, IMAP, Gmail API가 각자 지금 시도를 기억한다. 런타임의 OTP 세션 하나가 맡는다.
11. **테스트만 쓰는 config.yaml 저장·불러오기를 옮기지 않음**: 통째로 덮어쓰는 저장과 읽기 전용 불러오기는 앱이 쓰지 않는다.
