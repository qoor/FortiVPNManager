# FortiVPNManager 구조 설계

# 0. 문서 정보
- 목적: 재구현 대상의 기능 구성, 아키텍처 구성, 디렉토리 구성 기준
- 범위
  - 포함: 영역·묶음·헤더의 구분과 역할, 디렉토리와 crate 구성
  - 제외: 공개 인터페이스, 세부 동작, 헤더 안 세부 모듈

| 용어 | 뜻 |
|---|---|
| 영역 | 실행 주체와 역할로 나눈 최상위 구분 |
| 묶음 | 다루는 대상이 같은 헤더의 모음 |
| 헤더 | 기능 단위 |
| 총괄 구현체 | 묶음 또는 헤더의 기능을 총괄하는, 그 이름을 가진 구현체 |
| 메인 서비스 | 서비스 관리의 총괄 구현체 |
| 진입점 | 실행 파일 또는 라이브러리의 시작 지점 |
| 연결 | 사용자가 보는 VPN 하나 |
| 세션 | 연결을 이루는 서버 로그인과 터널 한 벌 |
| 채널 | 앱과 Helper 사이의 통신 경로 |

# 1. 영역

| 영역 | 디렉토리 | 실행 주체 | 역할 |
|---|---|---|---|
| 시스템 영역 | `system` | Helper 서비스 | 사용자 영역 요청의 실제 처리 |
| 사용자 영역 | `user` | 앱 | VPN 연결 제어, 사용자 표시와 입력 |
| 공용 | `common` | 없음 | 두 영역이 함께 쓰는 정의와 OS 호출 |

- VPN 연결 제어의 주체: 사용자 영역
- Helper 서비스: 상시 실행
- 앱: 사용자 종료 가능
- 앱 미실행 중 재연결: 없음
- 공용 배치 기준: 두 영역이 함께 쓰는 것 한정
  - 한 영역 전용 구조: 해당 영역 crate에 배치

# 2. 구성 단계

| 단계 | 단위 | 대표 파일 | 내용 |
|---|---|---|---|
| 영역 | 디렉토리 | 없음 | 시스템, 사용자, 공용 |
| 묶음 | crate | `lib.rs` | 연결 관리, 프로토콜 관리, 서비스 관리 |
| 헤더 | 모듈 | `mod.rs` | 헤더의 총괄 구현체 |
| 세부 기능 | 모듈 파일 | 없음 | 총괄 구현체가 쓰는 기능 |

| 묶음 | 다루는 대상 |
|---|---|
| 연결 관리 | VPN 연결 |
| 프로토콜 관리 | 앱과 Helper 사이 통신의 구현체, 메인 서비스 메시지 |
| 서비스 관리 | 서비스로 실행되는 프로그램 자신 |

- 총괄 구현체: 대상마다 그 이름의 구현체 최소 하나
  - 적용 대상: 서비스 관리, 헤더, 로그 관리, UI, UI의 세 기능
  - 서비스 관리의 총괄 구현체: 메인 서비스
- 세부 기능: 헤더 디렉토리 안 모듈 파일로 추가
- 기능 사이 참조: 없음
  - 의존이 필요해지는 경우: 사용자 검토 요청
- 기능의 실행: 기능마다 전용 스레드와 작업 큐
- 기능에서 메인 서비스로의 전달: 메인 서비스 메시지
- 메인 서비스에서 기능으로의 전달: 기능의 작업
- 메시지와 작업: 처리에 필요한 내용을 모두 담은 요청 단위
- 메인 서비스: 메시지 종류별 고정 규칙으로 전달, 판정 없음
- 시각에 따른 작업: 해당 기능이 수행
- 로그 관리: 메인 서비스를 거치지 않는 호출
- 단계의 예외
  - 메인 서비스: 헤더 없이 서비스 관리 crate의 `lib.rs`에 배치
  - 로그 관리: 묶음 밖 crate
  - UI: 묶음 밖, Rust crate 아님

# 3. 실행 흐름

```
Helper   main 진입 (system/helper)
           메인 서비스 생성과 준비 (system/service)
             기능 생성과 준비
             기능별 스레드 실행
           메인 루프
           기능 중지
         main 종료

앱       main 진입 (user/app의 Swift·C#)
           시작 호출 (user/app의 Rust 통로)
             메인 서비스 생성과 준비 (user/service)
               기능 생성과 준비
               기능별 스레드 실행
             메인 루프 (전용 스레드)
           UI 준비와 실행 (user/ui)
           중지 호출 (user/app의 Rust 통로)
             기능 중지
         main 종료
```

- 진입점: 메인 서비스 생성과 종료 한정
- 메인 서비스: 기능 생성과 실행, 기능 사이 메시지 전달, 서비스 시작과 종료의 경계
- 메인 루프: 메시지가 들어오면 전달, 그 밖에는 대기
- 앱 진입점의 Rust 통로: 메인 서비스 시작·종료 호출, UI와 메인 서비스 사이의 통로

# 4. 시스템 영역

| 묶음 | 디렉토리 | 헤더 | 모듈 | 인터페이스 예시 |
|---|---|---|---|---|
| 서비스 관리 | `service` | VPN Helper 서비스 | `lib.rs` | 메인 서비스 |
| 서비스 관리 | `service` | Helper 운영 상태 관리 | `operation_state` | 운영 상태 확인 기능, 운영 상태 저장 기능 |
| 서비스 관리 | `service` | 업데이트 관리 | `update` | 업데이트 준비 기능, 업데이트 설치 기능 |
| 연결 관리 | `connection` | VPN 연결 관리 | `vpn` | VPN 멀티 세션 관리 기능, VPN 세션 연결 관리 기능 |
| 연결 관리 | `connection` | 네트워크 관리 | `network` | VPN 터널 설정 기능, 예외 경로 관리 기능, 네트워크 상태 관리 기능 |
| 프로토콜 관리 | `protocol` | 앱 연동 관리 | `app_link` | 앱 요청 채널 관리 기능, Helper 전파 채널 관리 기능, Helper 로그 채널 관리 기능, 앱 요청 분석 기능 |
| 묶음 밖 | `log` | 로그 관리 | `lib.rs` | VPN 로그 기록 기능, Helper 로그 기록 기능, 시스템 로그 기록 기능 |

- 진입점: `helper`
- 메인 서비스 메시지: `protocol`의 `service` 모듈
- OpenConnect 바인딩: `openconnect`
  - 사용 영역: 시스템 영역 한정

# 5. 사용자 영역

| 묶음 | 디렉토리 | 헤더 | 모듈 | 인터페이스 예시 |
|---|---|---|---|---|
| 서비스 관리 | `service` | VPN 앱 서비스 | `lib.rs` | 메인 서비스 |
| 서비스 관리 | `service` | 설정 관리 | `config` | 설정 로드 기능, 설정 로드 기능 (외부), 설정 저장 기능, 앱 상태 유지 기능 |
| 서비스 관리 | `service` | 업데이트 관리 | `update` | 업데이트 확인 기능, 업데이트 수행 기능 |
| 연결 관리 | `connection` | VPN 연결 제어 | `control` | VPN 연결 제어 기능 |
| 연결 관리 | `connection` | 연결 정책 관리 | `policy` | 연결 스케줄 관리 기능, 유휴 상태 관리 기능 |
| 연결 관리 | `connection` | 인증 관리 | `auth` | VPN 계정 관리 기능, MFA(OTP) 인증 기능 |
| 프로토콜 관리 | `protocol` | Helper 연동 관리 | `helper_link` | 앱 요청 채널 관리 기능, Helper 전파 채널 관리 기능, Helper 로그 채널 관리 기능, Helper 서비스 관리 기능 |
| 묶음 밖 | `log` | 로그 관리 | `lib.rs` | VPN 로그 기록 기능, 앱 로그 기록 기능, 시스템 로그 기록 기능 |
| 묶음 밖 | `ui` | UI | OS별 디렉토리 | 메뉴바(트레이) 기능, 설정창 기능, 알림 기능 |

- 진입점: `app`
  - 구성: Rust 통로, Swift main, C# main
  - 범위: 진입 코드 한정
- 메인 서비스 메시지: `protocol`의 `service` 모듈
- Helper 서비스 준비
  - 서비스 등록과 자동 시작 설정: 설치 패키지
  - 앱 실행 시 Helper 기동: Helper 연동 관리
  - 기동 호출 시점: VPN 앱 서비스
  - 승인 대기 중 기동 재시도: Helper 연동 관리
- UI
  - 역할: 표시와 입력 한정
  - UI의 총괄 구현체: OS 디렉토리 최상위, 세 기능의 생성과 메인 서비스 사이 전달
  - UI 외 헤더의 UI 참조: 없음
  - 구성: OS 이름 아래 세 기능 디렉토리를 두 OS에 동일 배치
  - 문장 생성 코드: `ui/OS/Localization`
- 앱 리소스: `resources`
  - 구성: OS 이름 아래 유형별 디렉토리
  - 번역 리소스 사용처: UI 한정

# 6. 공용

| crate | 디렉토리 | 모듈 | 내용 |
|---|---|---|---|
| 공용 기반 | `base` | `constants` | 공유 상수 |
| 공용 기반 | `base` | `validation` | 유효성 검사 |
| 공용 기반 | `base` | `protocol` | 앱과 Helper 사이 통신 메시지 구조 |
| OS API | `os-api` | `macos` | macOS API |
| OS API | `os-api` | `windows` | Windows API |

- 프로토콜 구조: `common/base`의 `protocol`
- 프로토콜 구현체: 두 영역의 `protocol` crate
  - 영역별 역할: 클라이언트 또는 서버

# 7. 디렉토리 구조
- 번호: `docs/FEATURES.md`의 헤더 번호
- 헤더 아래 들여 쓴 이름: `docs/FEATURES.md`의 기능 항목
  - 자리: 해당 헤더의 총괄 구현체

```
Cargo.toml                       Rust workspace
project.yml                      Xcode 프로젝트 정의
Config/                          Xcode 설정 파일
FortiVPNManager.slnx             Visual Studio 솔루션
*.csproj                         C# 프로젝트

system/                          시스템 영역 (Helper 서비스)
  helper/                        진입점
  service/                       서비스 관리
    src/
      lib.rs                     메인 서비스: VPN Helper 서비스
      operation_state/mod.rs     4. Helper 운영 상태 관리
                                   운영 상태 확인 기능
                                   운영 상태 저장 기능
      update/mod.rs              6. 업데이트 관리
                                   업데이트 준비 기능
                                   업데이트 설치 기능
  connection/                    연결 관리
    src/
      lib.rs
      vpn/mod.rs                 1. VPN 연결 관리
                                   VPN 멀티 세션 관리 기능
                                   VPN 세션 연결 관리 기능
      network/mod.rs             3. 네트워크 관리
                                   VPN 터널 설정 기능
                                   예외 경로 관리 기능
                                   네트워크 상태 관리 기능
  protocol/                      프로토콜 관리
    src/
      lib.rs
      app_link/mod.rs            2. 앱 연동 관리
                                   앱 요청 채널 관리 기능
                                   Helper 전파 채널 관리 기능
                                   Helper 로그 채널 관리 기능
                                   앱 요청 분석 기능
      service/mod.rs             메인 서비스 메시지
  log/                           5. 로그 관리
    src/
      lib.rs                       VPN 로그 기록 기능
                                   Helper 로그 기록 기능
                                   시스템 로그 기록 기능
  openconnect/                   OpenConnect 바인딩

user/                            사용자 영역 (앱)
  app/                           진입점
    src/                         Rust 통로
    macos/                       Swift main
    windows/                     C# main
  service/                       서비스 관리
    src/
      lib.rs                     메인 서비스: VPN 앱 서비스
      config/mod.rs              3. 설정 관리
                                   설정 로드 기능
                                   설정 로드 기능 (외부)
                                   설정 저장 기능
                                   앱 상태 유지 기능
      update/mod.rs              6. 업데이트 관리
                                   업데이트 확인 기능
                                   업데이트 수행 기능
  connection/                    연결 관리
    src/
      lib.rs
      control/mod.rs             1. VPN 연결 제어
                                   VPN 연결 제어 기능
      policy/mod.rs              7. 연결 정책 관리
                                   연결 스케줄 관리 기능
                                   유휴 상태 관리 기능
      auth/mod.rs                4. 인증 관리
                                   VPN 계정 관리 기능
                                   MFA(OTP) 인증 기능
  protocol/                      프로토콜 관리
    src/
      lib.rs
      helper_link/mod.rs         2. Helper 연동 관리
                                   앱 요청 채널 관리 기능
                                   Helper 전파 채널 관리 기능
                                   Helper 로그 채널 관리 기능
                                   Helper 서비스 관리 기능
      service/mod.rs             메인 서비스 메시지
  log/                           8. 로그 관리
    src/
      lib.rs                       VPN 로그 기록 기능
                                   앱 로그 기록 기능
                                   시스템 로그 기록 기능
  ui/                            5. UI
    macos/
      AppUI.swift                UI
      MenuBar/                     메뉴바(트레이) 기능
      Settings/                    설정창 기능
      Notification/                알림 기능
      Localization/              문장 생성 코드
    windows/
      AppUI.cs                   UI
      MenuBar/                     메뉴바(트레이) 기능
      Settings/                    설정창 기능
      Notification/                알림 기능
      Localization/              문장 생성 코드
  resources/                     앱 리소스
    macos/
      Localization/              번역
      Icons/                     아이콘
    windows/
      Localization/              번역
      Icons/                     아이콘

common/                          공용
  base/                          공용 기반
    src/
      lib.rs
      constants/mod.rs           공유 상수
      validation/mod.rs          유효성 검사
      protocol/mod.rs            통신 메시지 구조
  os-api/                        OS API
    src/
      lib.rs
      macos/mod.rs               macOS API
      windows/mod.rs             Windows API

packaging/                       설치 패키지
  macos/
    installer/                   설치 패키지 정의
    scripts/                     빌드·설치 스크립트
    resources/                   설치 패키지 리소스
  windows/
    installer/                   설치 패키지 정의
    scripts/                     빌드·설치 스크립트
    resources/                   설치 패키지 리소스
  openconnect/
    scripts/                     OpenConnect 빌드 스크립트
```

# 8. crate

| 영역 | crate | 디렉토리 | 산출물 |
|---|---|---|---|
| 시스템 | `fvm-system-helper` | `system/helper` | 실행 파일 |
| 시스템 | `fvm-system-service` | `system/service` | 라이브러리 |
| 시스템 | `fvm-system-connection` | `system/connection` | 라이브러리 |
| 시스템 | `fvm-system-protocol` | `system/protocol` | 라이브러리 |
| 시스템 | `fvm-system-log` | `system/log` | 라이브러리 |
| 시스템 | `fvm-system-openconnect` | `system/openconnect` | 라이브러리 |
| 사용자 | `fvm-user-app` | `user/app` | 정적 라이브러리 |
| 사용자 | `fvm-user-service` | `user/service` | 라이브러리 |
| 사용자 | `fvm-user-connection` | `user/connection` | 라이브러리 |
| 사용자 | `fvm-user-protocol` | `user/protocol` | 라이브러리 |
| 사용자 | `fvm-user-log` | `user/log` | 라이브러리 |
| 공용 | `fvm-common-base` | `common/base` | 라이브러리 |
| 공용 | `fvm-common-os-api` | `common/os-api` | 라이브러리 |

- 이름 규칙: `fvm-영역-디렉토리`
- crate 내부 배치: Cargo 표준 `src/`
- crate 사이 순환 의존: 불가
- 묶음 사이 의존이 필요해지는 경우: 사용자 검토 요청

# 9. 그 밖의 배치

| 대상 | 자리 | 구분 방식 |
|---|---|---|
| Rust workspace, Xcode 프로젝트 정의, Visual Studio 솔루션, C# 프로젝트 | 저장소 최상위 | 없음 |
| Xcode 설정 파일 | 최상위 `Config` | 없음 |
| 앱 리소스 | `user/resources` | OS 이름, 유형 |
| 설치 패키지 | `packaging` | OS 이름, `installer`·`scripts`·`resources` |

| 언어 | 담당 | 자리 |
|---|---|---|
| Swift | macOS UI, macOS 앱의 main | `user/ui/macos`, `user/app/macos` |
| C# | Windows UI, Windows 앱의 main | `user/ui/windows`, `user/app/windows` |
| Rust | 그 밖의 전부 | `system`, `user`의 나머지, `common` |

| 언어 | 시험 자리 |
|---|---|
| Rust | 헤더 디렉토리 안 시험 파일, crate의 `tests` |
| Swift | 기능 디렉토리 안 `Tests` |
| C# | 기능 디렉토리 안 `Tests` |

- 시험 배치 기준: 대상 코드와 같은 디렉토리
- Swift·C# 제품 대상의 `Tests` 포함: 없음

# 10. 구현 단계 결정 항목
- 헤더 안 세부 모듈의 수와 이름
- OS API의 OS별 모듈을 쓰는 쪽에 내주는 방식
- macOS의 Helper 기동 호출을 Rust에서 수행할 수 있는지 확인
- 최상위 C# 프로젝트의 중간 산출물 분리
- 기능 실행 함수의 비동기 전환 여부
- Rust 통로와 Swift·C# 사이의 호출 형식
- 기능 중지의 순서와 시간 제한
  - 포함: 서비스 중지 때의 터널 설정 원복
- 기능 준비 실패의 처리
- 로그 채널 송신의 실행 위치

# 11. 기능 목록
- 별도 파일: `docs/FEATURES.md`
