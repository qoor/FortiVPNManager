# FortiVPNManager 기능 명세

# 0. 문서 정보
- 목적: 구현과 검증의 기준이 되는 기능 명세
- 범위
  - macOS + openfortivpn, Windows + openconnect
  - openconnect 처리: OS와 무관한 규칙(macOS·Linux 확장 대비)
  - Linux: 자리만 확보
- OS 표기
  - Keychain: 각 OS의 비밀값 저장소(Windows는 Credential Manager)
  - "이 Mac": 이 기기
  - openfortivpn 규칙 중 VPN client와 무관한 것은 openconnect에도 동일 적용
  - OS·VPN client별 차이만 별도 기재

| 용어 | 뜻 |
|---|---|
| Connection Manager | 연결 state machine과 그 실행 부분(규칙은 판단 코어, 실행은 런타임) |
| 시도 | 연결됨 또는 실패로 끝나는 연결 시도 한 번, 시도 ID로 구분 |
| 실행 | Helper에서 VPN client 프로세스 하나가 살아 있는 기간, 동시에 하나 |
| 진행 단계 (milestone) | 실행됨 → 게이트웨이 연결 → OTP prompt → OTP 제출 → 인증됨 → 터널 올라옴 |
| 종료 사실 | 실행 종료 시 Helper의 기록(가장 멀리 간 단계, 종료 주체, 터널 유지 시간 등) |
| 안정 세션 | 정해진 시간 이상 유지된 터널 |
| 멈춤 | 자동 재시도 없이 사용자의 [연결]·[지금 재연결]을 기다리는 상태 |
| 도메인 구역 | UI로 보내는 상태 단위: 연결, 스케줄, 준비 상태·Helper, Gmail·Google 계정, 예외 규칙, 설정 |
| port | 공통 부분이 OS 기능을 요청하는 인터페이스, OS별 구현 |

# 1. 시스템 구성

## 1.1 전체 구성

![전체 구성](images/system-overview.svg)

- 그림
  - 앱과 Helper: 별도 프로세스
  - 화살표: 서로 주고받는 내용
  - 공통 계약: 양쪽 공통 약속이라 그림에서 제외
  - 기준: macOS 구성
  - Windows: 앱 쪽 동일, Helper 쪽은 세 모듈과 Helper OS 영역 (Windows)이 Helper 서비스 하나로 실행

| 구성 요소 | 위치 | 역할 |
|---|---|---|
| UI (macOS) | 앱 | 메뉴바와 설정창, 사용자에게 보이는 모든 문장과 번역 |
| OS 연동 (macOS) | 앱 | OS 기능 제공, 시스템 이벤트 전달, Helper와의 통신 |
| 런타임 | 앱 | 조율, 타이머, OTP 세션, Google 계정, 설정·상태·로그 파일, 예외 규칙 동기화 |
| 판단 코어 | 앱 | Connection Manager 규칙과 데이터 형식 해석, 바깥과 직접 통신 없음 |
| 공통 계약 | 앱·Helper | 앱과 Helper가 주고받는 내용, VPN client 처리가 지킬 약속 |
| backend adapter | Helper(일부는 앱) | VPN client별 처리(openfortivpn, openconnect) |
| process 모듈 | Helper | VPN client 확인과 실행, 비밀번호·OTP·cookie 입력, 출력 처리, 시간 제한, 종료, logout |
| recovery 모듈 | Helper | 실행 기록과 예외 route 기록, 이전 Helper가 남긴 프로세스와 예외 route 정리, route 복구 요청 |
| entry 모듈 | Helper | 앱 요청 처리, 실행과 예외 route의 주인 판단, Helper 수명 |
| Helper OS 영역 (macOS) | Helper | 앱 연결 확인, Helper 실행 관리와 자가 업데이트, route 복구와 예외 route 넣기·지우기 |
| UI (Windows) | 앱 | 알림 영역 아이콘과 설정창, 사용자에게 보이는 모든 문장과 번역 |
| OS 연동 (Windows) | 앱 | OS 기능 제공, 시스템 이벤트 전달, Helper와의 통신 |
| Helper OS 영역 (Windows) | Helper | Helper 서비스로 실행, 세 모듈 사용, 서비스 수명, 앱 연결 확인, route 정리, 예외 route 넣기·지우기 |
| 설치 패키지 (Windows) | 설치·제거 때 | 앱, Helper, OpenConnect 설치와 업그레이드, Helper 서비스 등록, 제거 때의 정리 |
| openfortivpn | Helper가 실행 | macOS의 VPN client와 pppd |
| openconnect | Helper가 실행 | Windows의 VPN client와 route 설정 스크립트(앱과 함께 설치) |
| Gmail · Google | 외부 | OTP 메일, 계정 인증 |
| config.yaml · state.json · 로그 파일 | 사용자 디스크 | 설정, 기기별 상태, 실행 로그 |
| macOS · Windows | OS | 비밀값 저장, 서비스 실행, 전원·화면·네트워크 이벤트, routing table |

- Helper: 관리자 권한으로 VPN client를 실행하는 별도 프로세스
  - 구성: entry·process·recovery 모듈(두 OS 공통) + 해당 OS의 Helper OS 영역
- (macOS)·(Windows) 표기: 해당 OS 전용 구성 요소, 표기 없는 것은 모든 OS 공통
- 본문: OS 표기 생략

## 1.2 구성 요소 사이의 연결
- 앱 안
  - UI → 런타임: 사용자 조작 명령
  - 런타임 → UI: 바뀐 상태, 새 로그
  - 런타임 → OS 연동: OS 기능 요청
  - OS 연동 → 런타임: 결과, 시스템 이벤트
- 앱 ↔ Helper
  - 서로 확인한 연결 사용
  - 앱 → Helper: 실행 요청, OTP, 연결 해제
  - Helper → 앱: 진행 단계, 출력, 종료 보고
  - 주고받는 내용: 공통 계약 기준
- Helper 안
  - Helper OS 영역 → entry 모듈: 받은 요청 전달
  - entry 모듈: 요청별 허용 여부 판단
  - VPN client 관련 처리: process·recovery 모듈에 위임
  - backend adapter 사용: process·recovery 모듈 한정, entry 모듈의 직접 접근 금지
  - backend adapter와 실행 기록 폴더 선택: Helper의 OS별 시작 부분
  - 이벤트·출력: 순서대로 앱에 전달
- 바깥
  - Gmail·Google 통신과 파일 처리: 런타임
  - 판단 코어 사용: 런타임 경유 한정

# 2. 구성 요소별 설명

## 2.1 공통 계약
- 설명: 앱 쪽·Helper 쪽·backend adapter의 공통 약속, 앱과 Helper 양쪽 메시지 작성·해석 기준
- 역할
  - 실행 요청, 진행 단계, prompt 종류, 종료 사실, Helper 오류의 정의
  - VPN client 처리의 필수 기능 목록, 메시지 송수신 규칙, IPv4 값 규칙
- 모르는 것: 통신 수단, 프로세스, 파일, 특정 VPN client
- 제공 기능: 없음(F-RT-02, F-VR-05가 쓰는 정의 제공)

## 2.2 판단 코어
- 설명: 앱의 모든 판단과 데이터 형식 해석, 바깥과 직접 통신 없음
  - 같은 입력·순서에 항상 같은 결과
- 역할
  - Connection Manager 규칙, 표시 값, 준비 상태 판정
  - 예외 규칙 계산, config.yaml·IMAP·Gmail API·OAuth·OTP 메일 형식
- 모르는 것: 파일, 네트워크, 타이머 실행, VPN client 이름, 화면 문장
- 제공 기능: F-CM-01~11, F-CF-01, F-BR-01, F-BR-03

## 2.3 런타임
- 설명: 앱 안의 조율과 OS 무관 외부 작업, 판단 코어가 정한 작업 실행
- 역할
  - 이벤트 처리, 타이머, Helper 요청 순서, 도메인 구역 전송
  - OTP 세션, Google 계정, config.yaml·state.json·로그 파일, 예외 규칙 동기화
- 모르는 것: 화면 문장, OS별 기능 구현, VPN client 프로세스
- backend adapter 사용 범위: 설정 가져오기, 인증서 값 형식, 계정 값 확인 한정
- 제공 기능: F-CF-02~04, F-OT-01~05, F-GA-01~02, F-BR-02, F-SR-01~02, F-RT-01~02

## 2.4 backend adapter
- 설명: 특정 VPN client 전용 처리 모음, openfortivpn·openconnect의 같은 인터페이스 사용
- 역할
  - 실행 파일·버전 확인, 실행 요청 검증, 설정 파일·argv 생성
  - 출력 해석, 종료 유예 시간, 세션 cookie·logout 요청, 기존 설정 읽기, 인증서 값 형식
  - 여러 OS용 VPN client의 OS별 차이(실행 파일 위치, 환경 변수, 일부 출력 문구)
- 모르는 것: 프로세스 생성·종료 수단, 앱과의 통신 수단, 재시도 판단
- 제공 기능: F-VR-01, F-VR-03

## 2.5 process 모듈
- 설명: Helper 안의 VPN client 담당, client 확인과 시도 하나의 실행 준비부터 종료까지 관리
  - 로그인·터널 분리 client: 두 프로세스 순차 실행, 강제 종료 후 logout 요청
  - VPN client별 차이: backend adapter에 위임
- 역할
  - client 확인, 실행 명세·설정 파일 준비, OTP 형식 확인
  - 프로세스 실행, stdin 입력, 출력 줄 조립
  - 단계별 시간 제한, 종료, logout, 종료 사실 기록
  - Windows Helper 시작 시 시간 제한 없는 client 확인 1회
- 모르는 것: VPN client 이름·출력 문구, route 복구 방법, 앱과의 통신, 실행 요청 사용자
- 제공 기능: F-VR-01, F-VR-02, F-VR-04, F-VR-05, F-WN-03의 시작 시 client 확인

## 2.6 recovery 모듈
- 설명: Helper 비정상 종료 대비 실행·예외 route 기록, 다음 Helper 시작 시 그 기록으로 정리
  - VPN client 관련 처리: process 모듈이 넘긴 backend adapter에 위임
- 역할
  - 실행 기록(실행 중 client와 그 하위 프로세스, 게이트웨이), 예외 route 기록(Helper가 넣고 아직 지우지 않은 route)
  - 이전 Helper가 남긴 프로세스 정리, 남은 파일 삭제
  - route 복구 요청: 이전 Helper의 실행, route를 되돌리지 않고 끝난 실행
  - 이전 Helper가 남긴 예외 route 삭제 요청
- 모르는 것: route 수정·추가·삭제 방법, 앱과의 통신, 실행 요청 사용자
- 제공 기능: F-VR-06, F-BR-02의 예외 route 기록

## 2.7 entry 모듈
- 설명: Helper의 OS 무관 앞단, 앱 요청 접수와 허용 판단
  - VPN client 관련 처리: process·recovery 모듈에 위임
  - 상태 응답: process 모듈 제공 정보 기반
- 역할
  - 앱 연결, 실행 하나와 그 주인 관리, 응답 생성
  - 앱 연결별 원하는 예외 route와 Helper가 넣은 예외 route 관리, 넣고 지울 route의 OS 쪽 전달
  - 이전 Helper 잔여물 정리·route 복구 완료 전 앱 연결 차단, 실행이 요청한 route 복구의 OS 쪽 전달
  - Helper 종료 판단, Helper 로그
- 모르는 것: 앱과의 통신 수단, 연결 확인, route 수정·추가·삭제 방법, VPN client(backend adapter 직접 접근 불가), OS의 프로세스·파일 권한 처리
- 제공 기능: F-VR-06, F-VR-07, F-BR-02의 Helper 쪽

## 2.8 UI (macOS)
- 설명: 도메인 구역의 화면 표시, 사용자 조작의 명령 전송, 사용자에게 보이는 모든 문장과 번역 생성
- 역할
  - 메뉴바 아이콘, 메뉴, 설정창 pane
  - 종류·값 기반 문장 생성과 번역, 상대 시간 표시
- 모르는 것: 연결 규칙(보일 값과 버튼 상태는 구역 값 그대로)
- 제공 기능: F-SW-01

## 2.9 OS 연동 (macOS)
- 설명: 런타임이 요청한 OS 기능의 macOS 제공, 시스템 이벤트 전달
- 역할
  - 비밀값 저장, Helper 등록·통신, 알림, 브라우저 열기, 앱 수명
  - 시스템 이벤트, 네트워크 정보
- 모르는 것: 이벤트의 의미(자리 비움 같은 조합·판단은 런타임)
- 제공 기능: F-SW-02

## 2.10 Helper OS 영역 (macOS)
- 설명: Helper 프로세스의 OS 의존 부분, 앱 쪽 OS 연동에 대응, 나머지는 entry 모듈에 전달
- 역할
  - 앱 연결 접수와 서명 확인, Helper 실행 관리와 자가 업데이트
  - route 복구, entry 요청에 따른 예외 route 추가·삭제와 주 네트워크 조회
- 모르는 것: 앱 요청 내용(버전 확인 제외), VPN client 실행, 예외 route의 주인
- 제공 기능: F-SW-03

## 2.11 UI (Windows)
- 설명: UI (macOS)와 같은 역할의 Windows 구현
  - 사용자에게 보이는 구성(창, 메뉴, pane과 순서, pane 선택 방식, pane 안 항목의 섹션과 순서): macOS와 동일
  - 컨트롤: Windows 기본 컨트롤
- 역할
  - 알림 영역 아이콘과 메뉴, 설정창 pane
  - 종류·값 기반 문장 생성과 번역, 상대 시간 표시
- 모르는 것: 연결 규칙(보일 값과 버튼 상태는 구역 값 그대로)
- 제공 기능: F-WN-01, F-WN-05의 오류 창

## 2.12 OS 연동 (Windows)
- 설명: 런타임이 요청한 OS 기능의 Windows 제공, 시스템 이벤트 전달, 런타임과의 메시지는 macOS와 동일
- 역할
  - 앱 시작 시 Helper 서비스 시작과 대기
  - 비밀값 저장, Helper 서비스와의 통신, 알림, 브라우저 열기, 앱 수명
  - 시스템 이벤트, 네트워크 정보
- 모르는 것: 이벤트의 의미, 앱과 Helper가 주고받는 내용(버전 확인 제외)
- 제공 기능: F-WN-02, F-WN-05

## 2.13 Helper OS 영역 (Windows)
- 설명: Windows의 Helper OS 영역, Helper 서비스로 실행, entry·process·recovery 모듈 사용, macOS Helper OS 영역의 일을 직접 수행
- 역할
  - 서비스 수명(실행 중 알림 전 process 모듈의 VPN client 확인 포함), 앱 연결 접수·확인
  - process 모듈용 backend adapter(openconnect)와 실행 기록 폴더 선택
  - 게이트웨이 host route 정리, entry 요청에 따른 예외 route 추가·삭제와 주 interface 조회
- 모르는 것: VPN client 출력 문구, 재시도 판단, 예외 route의 주인
- 제공 기능: F-VR-06, F-VR-07, F-WN-03

## 2.14 설치 패키지 (Windows)
- 설명: Windows 앱의 설치·업그레이드·제거용 MSI 설치 패키지, 관리자 권한 실행
- 역할
  - 앱, Helper, OpenConnect 설치와 업그레이드
  - Helper 서비스, 이벤트 로그 출처, 시작 메뉴 바로 가기 등록
  - 제거 시 앱이 PC에 넣은 것(로그인 시 열기, Helper와 실행 기록) 정리
- 모르는 것: 연결과 VPN client 실행, config.yaml·상태 파일 내용
- 제공 기능: F-WN-04

# 3. 기능 명세

## 3.1 Connection Manager (F-CM)

### F-CM-01 사용자 요청 처리
- 설명: [연결]·[연결 해제]·[지금 재연결]·[종료]에 따른 원하는 상태 결정, 시도·세션의 시작과 종료
- 담당: 판단 코어, 런타임, OS 연동(종료 승인)
- 입력: 사용자 명령
- 출력: 원하는 상태, 시도 시작·중지, 멈춤 해제, 종료 승인
- 세부 기능
  1. [연결]: 멈춤·자동 재연결 중단 해제, 실패 기록 초기화 후 시도, 마지막 인증 실패 뒤 대기 시간 유지
  2. [연결 해제]: 진행 중인 시도·세션 종료, 재시도 대기 취소
  3. [지금 재연결]: 연결됨이면 세션 종료 후 새 연결, 재시도 대기 중이면 [연결]과 같은 동작, 시도 중·끊는 중이면 무시
  4. [종료]: 터널 종료와 로그 파일 쓰기 완료 후 종료 승인, Helper 무응답 시 정해진 시간 뒤 종료
  5. 앱 요청으로 끝낸 세션: 실패 미집계, 종료 판단은 응답이 아닌 Helper의 종료 보고 기준

### F-CM-02 시도 진행
- 설명: 준비 상태 확인 → OTP 세션 → 터널 시작 → OTP 제출 → 연결됨 순서의 시도 진행
- 담당: 판단 코어, 런타임
- 입력: 준비 상태, OTP 세션 결과, 진행 단계
- 출력: OTP 세션 시작, 실행 요청, OTP 제출, 연결됨
- 세부 기능
  1. 준비 상태 부족 시 연결 생략, 필요한 항목 표시
  2. VPN client보다 먼저 OTP 세션 시작, Gmail 인증 실패 시 VPN client 미실행과 멈춤
  3. 실행 요청: Keychain의 VPN 비밀번호와 시도 시작 시점의 계정 사용, 시도 중 바뀐 계정은 다음 시도부터 적용
  4. OTP prompt 시 OTP 찾기, 찾은 코드 1회 제출
  5. 비밀번호 전송 후 OTP 제출 없이 끝난 시도: 종료 방식과 무관하게 그 시도의 OTP 메일 찾기와 휴지통 이동(F-OT-04의 2)
     - 제외: 앱 종료, 잠자기, VPN 로그인 거부로 센 종료(F-CM-03의 7), OTP 시간 초과(F-OT-04의 3)
     - 다음 시도: 준비 상태 확인 후 그 찾기가 진행 중이고 정해진 시간 안이면, 메일 이동·찾기 종료·시간 경과 중 하나까지 OTP 세션 시작 보류
     - 보류 중 진행 단계: Gmail 연결 중
     - 보류 시간: 시도 전체 시간 제한 미포함, 실패 미집계
     - 찾는 동안 앱 활동 유지
  6. 터널이 올라오면 연결됨, 하부 경로 기억, 사전 재연결 시각 설정
  7. 시도 전체·OTP 대기의 시간 제한, 지난 시도의 늦은 결과 무시

### F-CM-03 종료 원인 분류
- 설명: 앱이 요청하지 않은 종료의 원인을 종료 사실로 판단
- 담당: 판단 코어
- 입력: 종료 사실, Helper 연결 끊김, 경로 변경, 깨어난 시각
- 출력: 인증서 재시도, 강제 종료, 서버 종료, 네트워크 오류, 인증 실패, 치명 중 하나
- 세부 기능(처음 맞는 하나 적용)
  1. 앱이 끝낸 실행(연결 해제, 사전 재연결, 잠자기, 앱 종료): 분류 제외
  2. Gmail 세션 실패로 끝낸 실행: 인증 문제면 치명, 그 밖은 네트워크 오류
  3. OTP 제출 전 인증서 거부: 즉시 1회 재시도, 실패 미집계, 다시 거부되면 네트워크 오류
  4. 강제 종료
     - 터널이 올라온 뒤 앱·Helper가 멈추지 않은 client의 signal 종료
     - Windows: 터널이 올라온 뒤 요청 없는 모든 종료
     - 연결됨·터널 단계 시도 중 Helper 연결 끊김
     - 유지 시간과 무관하게 인증 실패 미집계
     - 터널 전 signal 종료: 6~8의 단계 규칙 적용
  5. 요청 없이 끝난 안정 세션: 서버 종료
  6. 게이트웨이 도달 전 종료: 네트워크 오류
  7. 로그인 단계 종료: 인증 실패, 시도 중 네트워크 변경 또는 깨어난 직후면 네트워크 오류
  8. 이후 단계 실패(OTP 미수신·거부, 인증 뒤 터널 거부, 너무 빨리 끊김 등): 인증 실패

### F-CM-04 재시도와 실패 제한
- 설명: 연결 실패 시 재시도 여부·시점·멈춤 결정
- 담당: 판단 코어(규칙), 런타임(대기와 저장)
- 입력: 종료 원인 분류 결과, 사용자 명령
- 출력: 다음 시도 시각, 멈춤 상태, 실패 기록
- 세부 기능
  1. 네트워크 오류: 실패 미집계, backoff 후 재시도, 서버 종료는 잠시 뒤 재연결
  2. 강제 종료: 실패 미집계, 즉시 재연결, 원하는 상태·잠자기·네트워크 대기·스케줄 보류와 중단(F-CM-08)·자동 시도 상한은 다른 재시도와 동일 적용
  3. 즉시 재연결 후 안정 세션 없이 다시 강제 종료(연결 중 종료 포함): 네트워크 오류와 같은 backoff, 안정 세션 발생 또는 [연결]·[지금 재연결] 시 즉시 재연결 복귀, 이 상태 미저장
  4. 인증 실패 1회: 차단 해제 시간 대기 후 1회 재시도
  5. 멈춤 후 [연결]·[지금 재연결] 대기: 인증 실패 2회, Gmail 인증 실패, Helper의 실행 요청 수신 불가 오류
  6. 일정 시간 내 자동 시도 횟수 상한, 도달 시 멈춤, 사용자가 직접 시작한 시도는 제외
  7. 안정 세션 종료 시 연속 실패 0으로 초기화
  8. 실패 기록: 계정 단위 저장, 앱 재실행 후 유지

### F-CM-05 사전 재연결
- 설명: 서버가 세션을 끊기 전, 정해 둔 시각의 세션 종료와 재연결(break-before-make)
- 담당: 판단 코어, 런타임
- 입력: 연결 시각, `connection.session_max_minutes`, `connection.proactive_reconnect`
- 출력: 사전 재연결 시각, 재연결
- 세부 기능
  1. 시각: 연결 시각 + (세션 최대 시간 − 여유), 너무 이르면 최소 간격 뒤로 연기
  2. 그 시각에 세션 종료, 종료 보고 후 대기 없이 재연결, 실패 미집계
  3. 설정 변경 시 현재 세션의 시각 재계산, 끄면 취소
  4. 잠자는 동안과 자동 재연결 제한 중(F-CM-08) 미실행

### F-CM-06 잠자기와 깨어남
- 설명: 잠자기 중 터널 유지, 깨어난 뒤 상태에 따른 재연결 판단
- 담당: 판단 코어, 런타임, OS 연동
- 입력: 잠들기 직전, 깨어남, 깨어난 뒤의 관찰
- 출력: 유지 또는 재연결
- 세부 기능
  1. 잠들 때: 연결 중·재시도 대기 중이면 중단(실패 미집계), 연결됨이면 터널 유지
  2. 깨어난 뒤: 네트워크 복귀 대기 후 client 프로세스, 터널 interface, 하부 경로 관찰
  3. 연결된 채 깨어남: 다음 경우에만 재연결, 그 밖은 유지
     - 사전 재연결 시각 경과
     - 프로세스·터널 interface 소멸
     - 하부 경로 변경
     - "잠자기 후 재연결" 조건 충족
  4. 중단한 시도: 네트워크 복귀 후 새로 시작
  5. 연결 상태 확인 대상: client 프로세스와 터널 interface 한정

### F-CM-07 네트워크 변화
- 설명: 경로 끊김 시 시도 연기, 게이트웨이까지의 물리 경로 변경 시 터널 재연결
- 담당: 판단 코어, 런타임, OS 연동
- 입력: 경로 유무, 하부 경로
- 출력: 네트워크 대기, 재연결
- 세부 기능
  1. 하부 경로: 게이트웨이 IPv4로 가는 interface와 그 주소, 게이트웨이 IPv4는 host마다 1회 확인
  2. 경로 없음: 시도 보류, 복귀 시 시도
  3. 연결됨에서 하부 경로 변경: 재연결, 같은 경로로 복귀하면 동작 없음
  4. 시도 중 경로 변화: 기록 후 분류(F-CM-03)에 사용

### F-CM-08 스케줄과 자리 비움
- 설명: 정한 시간의 자동 연결, 그 시간 밖 자리 비움 시 자동 재연결만 제한
- 담당: 판단 코어, 런타임, OS 연동
- 입력: `connection.schedule`, 자리 비움 여부, 현지 시각
- 출력: 시작 시각의 연결, 재연결 보류·중단, 스케줄 전망
- 세부 기능
  1. 스케줄 시간: 선택한 요일마다 현지 시각의 시작~종료 구간 하나
  2. 시작 시각: [연결]과 같은 동작
  3. 스케줄로 인한 연결 종료 없음, 종료 시각 뒤에는 자동 재연결만 제한
  4. 자리 비움(화면 잠금, 디스플레이 잠자기, 시스템 잠자기) 중 재연결 보류, 복귀 시 재연결
  5. 자리 비움의 `away_minutes` 도달 시 자동 재연결 중단
     - 살아 있는 터널 유지, 그 세션 종료 뒤 재연결 없음
     - 해제: [연결], [지금 재연결], 다음 시작 시각
  6. 앱 시작 시 자동 연결: 스케줄 시간 안 한정, 스케줄 변경 시 즉시 연결·종료 없음

### F-CM-09 서버 인증서 자동 신뢰
- 설명: 게이트웨이 인증서의 무조건 자동 신뢰
- 담당: 판단 코어, backend adapter, 런타임
- 입력: 종료 사실의 인증서 값
- 출력: `vpn.trusted_certs` 또는 `vpn.trusted_cert_pins` 갱신, 즉시 재시도
- 세부 기능
  1. VPN client가 거부하며 출력한 인증서 값 저장, 다음 연결부터 전달
  2. VPN client별 목록
     - openfortivpn의 SHA-256 digest: `vpn.trusted_certs`
     - openconnect의 `pin-sha256:` 값: `vpn.trusted_cert_pins`
     - 현재 VPN client의 목록만 사용, 그 목록의 문제만 경고
     - 다른 목록은 그대로 유지, 형식이 맞지 않는 값은 파일 재작성 시 제외
  3. 새 값은 목록 앞에 추가, 최대 4개 유지
  4. 저장 조건: 현재 설정된 게이트웨이를 향한 실행 한정
  5. 서버 주소·port 변경 시 두 목록 초기화, 사용자가 직접 넣은 값은 그대로 사용
  6. openconnect: 저장된 값이 없어도 맞지 않는 자리표시 값 1개 전달

### F-CM-10 표시 결정
- 설명: 아이콘·메뉴·주의 표시·상태 정보의 표시 내용 결정, UI는 이 값 그대로 표시
- 담당: 판단 코어
- 입력: Connection Manager 상태
- 출력: 연결 구역과 스케줄 구역의 값
- 세부 기능
  1. 아이콘: 연결됨, 끊김, 연결·재연결 중, 오류(멈춤과 설정 필요 포함) 네 가지
  2. 버튼
     - [연결]: 원하는 상태가 끊김이거나 주의 표시가 있을 때 표시
     - [연결 해제]: 원하는 상태가 연결일 때 표시
     - [지금 재연결]: 연결됨·재시도 대기일 때만 활성
  3. 표시 값: 주의 표시(설정 필요 항목 또는 멈춤 이유), 연결 시각, 터널 IP, 다음 재연결·재시도 시각, 대기 이유, 진행 단계, 스케줄 전망
  4. 진행 중인 세션의 계정이 현재 설정과 다르면 "다음 연결부터 적용" 포함
  5. UI 문장용 시간 값 함께 전달

### F-CM-11 준비 상태 판정
- 설명: 연결 전 필요 항목 확인, 부족 시 필요한 항목 안내
- 담당: 판단 코어, 런타임
- 입력: config.yaml 유효성, 계정 값, Keychain 항목 유무, Helper 상태
- 출력: 준비됨 또는 빠진 항목
- 세부 기능
  1. 확인 순서: 설정 파일 오류, 계정 정보, VPN 비밀번호, Gmail 인증(앱 비밀번호 또는 Google 계정 연결), Helper 승인(Windows는 Helper 서비스 설치·실행), Helper 버전 일치
  2. Keychain: 항목 유무만 확인, 비밀값 미조회(접근 prompt 없음)
  3. Helper 빌드 불일치
     - macOS: 시도가 없으면 옛 Helper에 유휴 시 종료 요청
     - Windows: 재설치 안내
  4. 재확인 시점: 앱 활성화, 설정창 열림, 관련 설정·인증 변경, [Helper 등록] 후 일정 시간 반복(승인 추적)

## 3.2 설정 (F-CF)

### F-CF-01 config.yaml 읽기와 검증
- 설명: 사람이 편집하는 config.yaml의 엄격한 읽기, 오류 시 마지막 정상 설정 유지와 연결 차단
- 담당: 판단 코어, 런타임
- 입력: config.yaml
- 출력: 설정 값과 경고, 또는 줄·열이 붙은 오류 하나
- 세부 기능
  1. 적지 않은 키와 null: 기본값
     - 텍스트 오류: 크기 초과, 깨진 UTF-8, 읽을 수 없는 제어 문자, raw NEL·LS·PS
     - 구조 오류: 문법 오류, 여러 문서, 깊은 중첩, 중복 키, anchor·alias, merge 키, tag, 복합 키
     - 값 오류: YAML 1.1식 숫자·불리언(`0443`, `yes` 등), 따옴표 숫자, 타입·범위·형식 위반, 교차 규칙 위반, 서로 겹치는 예외 규칙 항목
  2. 경고 후 무시: 알 수 없는 키, 형식이 틀리거나 중복된 인증서 값, 최대 개수 초과 인증서 값
  3. 더 새로운 `schema_version`: 가장 먼저 거부, 그 파일의 덮어쓰기 금지
  4. 파일 없음: 기본값, symlink는 따라감
     - macOS: 권한이 0600이 아니면 경고
     - Windows: 사용자 profile의 기본 권한 사용
  5. 같은 파일의 두 OS 공용, 앱이 쓰는 설명 주석은 해당 OS 기준(비밀값 저장소, 네트워크와 router 명칭)

### F-CF-02 설정 저장
- 설명: 설정창 변경과 앱이 학습한 값의 config.yaml·Keychain 저장
- 담당: 런타임, 판단 코어, OS 연동(Keychain)
- 입력: 저장할 변경
- 출력: config.yaml, Keychain 항목
- 세부 기능
  1. config.yaml 쓰기 시점: 설정창 저장, 새 인증서 값, 가져온 route, OAuth client 변경, 설정 가져오기 한정
  2. 매번 최신 파일 위에 변경만 적용, 손 편집과 겹치면 재시도, 내용이 같으면 쓰기 생략
  3. 파일 오류 시 쓰기 금지, 정해진 템플릿으로 쓰므로 직접 단 주석 소멸
  4. 0600 권한의 원자적 쓰기, symlink 유지
  5. 계정 저장: 바뀐 칸만 쓰기, 저장된 host·username·주소를 키로 비밀번호의 Keychain 쓰기, 빈 비밀번호 칸은 기존 값 유지
  6. VPN client가 받지 않는 값(비밀번호 길이 초과 등): 저장 전 거부

### F-CF-03 변경 감지
- 설명: 사용자의 config.yaml 수정을 앱 재시작 없이 반영
- 담당: 런타임
- 입력: 파일 상태와 내용
- 출력: 바뀐 설정
- 세부 기능
  1. 확인 시점: 짧은 주기, 앱 활성화
  2. 내용 변경 시에만 반영, 앱이 쓴 내용과 수정 시각만 바뀐 경우 제외, 앱이 쓰기 직전의 손 편집 누락 금지
  3. 변경 전달 대상: Connection Manager, 준비 상태, 예외 규칙

### F-CF-04 기존 설정 가져오기
- 설명: 기존 VPN 설정에서 계정 값 가져오기, 원본 수정 금지
  - macOS: openfortivpn 설정
  - Windows: 공식 FortiClient의 SSL VPN 연결
- 담당: 런타임, backend adapter, OS 연동(Windows), UI
- 입력: `~/.config/openfortivpn/config` 또는 고른 파일, Windows는 FortiClient의 SSL VPN 연결
- 출력: config.yaml 계정 값, Keychain 비밀번호, 가져오기 보고
- 세부 기능
  1. 시점: config.yaml이 없는 첫 실행의 자동 가져오기, 일반 pane(파일 메뉴)의 직접 가져오기
  2. 가져오는 값: host, port, username, password, realm, trusted-cert, host 없으면 실패, port 없으면 443
  3. 비밀번호의 Keychain 저장, 원본의 평문 비밀번호 경고
  4. 알 수 없는 키와 쓸 수 없는 줄 보고
  5. Windows(FortiClient)
     - 연결의 `Server` 값(`host:port`, 포트 없으면 443)을 host와 port로 사용, scheme·경로가 붙은 주소는 실패
     - username·realm·비밀번호: 가져오지 않음, 기존 값 유지
     - gateway 변경 시 이전 gateway의 인증서 값 삭제
     - 첫 연결만 사용, 연결이 없으면 보고(첫 실행에서는 동작 없음)
     - FortiClient 설정은 읽기 전용, IPsec 연결 미대상

## 3.3 OTP (F-OT)

### F-OT-01 OTP 세션
- 설명: 한 시도 동안 Gmail 감시, 그 시도의 OTP 코드 찾기
- 담당: 런타임, 판단 코어
- 입력: Gmail 주소와 인증, OTP prompt 시각
- 출력: 준비, 코드, 실패(인증·네트워크)
- 세부 기능
  1. VPN client 실행 전 메일함 기준점 설정, 그 뒤 도착 메일만 후보
  2. OTP prompt 뒤 찾기 시작, prompt 시각(시계 차이 여유 포함) 이후 도착 메일만 사용
  3. 코드: 제목의 `AuthCode:` 뒤 6~8자리 숫자, 발신자 미확인
  4. 여럿이면 가장 늦게 도착한 메일 선택, 코드는 시도마다 1회 전달, 다른 시도의 결과 폐기

### F-OT-02 Gmail API 방식
- 설명: Google 계정 연결 기반 Gmail API의 OTP 메일 찾기, 기본 경로
- 담당: 런타임, 판단 코어
- 입력: access token, Gmail 주소
- 출력: F-OT-01의 세션 결과
- 세부 기능
  1. 연결된 Google 계정 주소가 설정과 다르면 인증 실패
  2. prompt 뒤 짧은 주기로 기준점 이후 추가 메일만 확인, 기준점이 오래돼 쓸 수 없으면 제목 검색으로 전환
  3. 제외: 스팸, 휴지통, 임시보관 메일
  4. 인증 오류는 즉시 인증 실패, 일시적 오류는 몇 차례 허용 후 네트워크 실패

### F-OT-03 IMAP 방식과 대체 경로
- 설명: 앱 비밀번호 기반 IMAP의 OTP 찾기, Gmail API 사용 불가 시 같은 시도 안의 전환
- 담당: 런타임, 판단 코어
- 입력: Gmail 주소, 앱 비밀번호, `otp.method`
- 출력: F-OT-01의 세션 결과
- 세부 기능
  1. 전체보관함(`\All`)의 UIDVALIDITY·UIDNEXT를 기준점으로 설정, IDLE 알림과 짧은 주기로 그 뒤 메일 찾기
  2. 자격증명 거부는 인증 실패, 그 밖의 오류와 UIDVALIDITY 변경은 네트워크 실패
  3. IMAP 전환 조건: Gmail API 방식에서 Google 계정이 없거나 API 준비 전 인증 실패, 앱 비밀번호 보유
  4. 전환 기록: 로그 한정

### F-OT-04 사용한 메일 정리와 스팸함 확인
- 설명: 사용한 OTP 메일의 휴지통 이동, OTP 미수신 시 스팸함 확인과 원인 안내
- 담당: 런타임
- 입력: OTP 제출, OTP를 쓰지 않고 끝난 시도의 정리 요청, OTP 시간 초과
- 출력: 휴지통 이동, 정리 완료 알림, 실패 이유 갱신
- 세부 기능
  1. OTP 제출 시 이번 세션에서 찾은 OTP 메일 모두 휴지통 이동
  2. 판단 코어의 정리 요청(F-CM-02의 5)
     - 세션 유지, 그 시도의 코드를 같은 규칙으로 계속 찾기
     - 기준 시각: OTP prompt, prompt가 없었으면 비밀번호 전송 시각
     - 찾으면 1과 같은 휴지통 이동, 판단 코어에 알린 뒤 세션 종료
     - 이동 실패 시 이유 로그, 알림 없음(다음 시도는 대기 시간까지 대기)
     - 세션 실패 시 판단 코어의 대기 종료, 대기 시간 경과 시 판단 코어의 세션 종료
  3. OTP 시간 초과 시 스팸함 확인, 코드가 있으면 실패 이유를 "OTP가 스팸함에 있음"으로 변경
  4. 정리 실패: 로그 한정

### F-OT-05 Gmail 연결 테스트
- 설명: 계정 pane에서 입력한 Gmail 설정의 즉시 동작 확인
- 담당: 런타임, UI
- 입력: 선택한 방식, 입력한 주소와 앱 비밀번호
- 출력: 성공, 인증 실패, 메일함 없음, 네트워크 오류
- 세부 기능
  1. 선택된 방식만 시험, 연결 상태와 실패 기록에 영향 없음
  2. IMAP: 로그인 후 필요한 메일함 확인
  3. Gmail API: client와 Google 계정 연결 확인, 계정 주소 비교

## 3.4 Google 계정 (F-GA)

### F-GA-01 Google 계정 연결과 해제
- 설명: 브라우저의 Google 로그인과 Gmail API 사용 허락, OAuth(PKCE)와 loopback 주소 수신
- 담당: 런타임, 판단 코어, OS 연동
- 입력: 저장된 Gmail 주소, OAuth client
- 출력: refresh token, access token, 연결 상태
- 세부 기능
  1. `127.0.0.1`의 임의 port 일회용 listener, 기본 브라우저의 인증 페이지 열기, state가 맞는 응답만 수신
  2. 확인 항목: refresh token, `gmail.modify` 권한, 저장된 Gmail 주소와 같은 계정, 하나라도 어긋나면 받은 token 철회와 이유 안내
  3. refresh token은 Keychain, access token은 메모리 한정, Google의 refresh token 거부 시 삭제와 "Google 계정 연결 필요" 표시
  4. [연결 해제]: token 삭제 후 Google에 철회 요청, 철회 실패 시에도 해제 유지

### F-GA-02 OAuth client
- 설명: 앱 식별용 OAuth Desktop client의 빌드 시 내장, 다른 client 가져오기 지원
- 담당: 런타임, 판단 코어, UI
- 입력: 내장 client, 고른 client JSON
- 출력: 사용 중인 client와 그 상태
- 세부 기능
  1. 가져온 client가 내장 client보다 우선, ID는 `otp.google_client_id`, secret은 Keychain
  2. 허용 형식: Google Cloud "데스크톱 앱" 한정
  3. client 변경 시 이전 client로 받은 token 삭제
  4. [내장 client 사용]: 가져온 client 삭제

## 3.5 예외 규칙 (F-BR)

### F-BR-01 적용 여부와 route 계산
- 설명: 사내망 직접 연결 시 지정한 host·대역의 VPN 대신 그 네트워크 router 경유
- 담당: 판단 코어
- 입력: `routes.networks`, `routes.bypass_vpn`, 주 네트워크의 router·주소, 앱의 VPN 유지 의사, VPN 서버 주소
- 출력: 주 네트워크가 가질 예외 route
- 세부 기능
  1. 적용 조건: 이 Mac의 IPv4 주소가 `routes.networks` 안에 있고 앱이 VPN을 켜 두려 할 때
     - 그 밖: 앱이 넣은 route 삭제
     - 이 목록의 앱 자동 추가 금지
  2. 앱이 VPN을 켜 두려 하는 때: 연결 중, 연결됨, 재연결을 위한 종료 중, 재시도 대기, 네트워크 대기
     - 제외: 사용자의 연결 해제, 실패로 인한 멈춤, 스케줄 밖 미연결, 자리 비움으로 인한 재연결 보류, 앱 종료
  3. 각 예외 대상의 현재 router 경유 route 생성, 같은 LAN 안의 대상 제외
  4. VPN 서버 주소의 예외 대상 불가
     - `vpn.host`가 IP 주소: 같은 host 하나를 config.yaml·설정창 행에서 오류 처리, 그 주소를 포함하는 대역은 허용
     - `vpn.host`가 이름: 동기화 시 그 이름의 알려진 주소(이번 실행이 연결한 gateway 주소, 네트워크 확인 때 찾은 주소)와 같은 host 대상의 route 생성 제외, 로그 1회, 설정창에 미적용 표시
  5. 예외 대상 지정: 설정창과 config.yaml 한정, 밖에서 넣은 route의 목록 가져오기 없음
  6. 삭제 대상: 앱이 넣은 route 한정, 밖에서 넣은 같은 대상 route 유지(F-BR-02)
  7. 목록에 있는데 사라진 route 재추가
  8. 한 목록 안의 항목 간 겹침 금지

### F-BR-02 동기화
- 설명: 앱이 VPN을 켜 두려 하는 동안에만 계산한 route의 routing table 추가, 그 밖이나 앱 종료 시 삭제, 실패 시 재시도와 알림
  - OS가 저장하는 route 목록(macOS additional routes, Windows persistent route) 미사용
- 담당: 런타임, entry 모듈, recovery 모듈, Helper OS 영역, OS 연동(알림)
- 입력: config.yaml, 연결 상태, 네트워크 설정 변경, Helper 연결 끊김, 앱 종료
- 출력: route 추가·삭제, Helper의 예외 route 기록, OS 알림
- 세부 기능
  1. 동기화 시점: 앱 시작, Helper 준비, Helper 연결 끊김 후, 앱의 VPN 유지 의사·VPN 서버의 알려진 주소 변경, `routes` 변경, 네트워크 설정 변경, 재시도 시각
     - Windows: 주소 변경 시와 주기적 추가 확인
     - 연결 시도 시작 시 즉시 동기화, 예외 route의 터널 route보다 먼저 추가
  2. config.yaml 없음·오류 시 route 미변경
  3. 읽기와 쓰기
     - Helper에서 주 네트워크와, 이 앱이 마지막으로 보낸 route 중 routing table에 있는 것 읽기
     - 계산 결과와 다르면 이 앱의 route 목록 전체 전송
     - 앱이 읽은 뒤 주 네트워크가 바뀌었으면 Helper의 변경 없음
     - 변경 후 결과 알림
  4. Helper의 route 관리
     - 앱 연결마다 원하는 route 기억, 어느 앱도 원하지 않는 route 삭제
     - 삭제 대상: Helper가 넣은 route 한정
     - 이미 있는 route: 추가 생략, 밖에서 넣은 route로 취급
     - macOS의 기존 route 기준: 대상·mask·gateway가 같은 static route 한정(cloned route(W)·interface 묶음 route(I) 제외), 같은 대상의 cloned host route는 삭제 후 추가
     - Windows의 기존 route 기준: 대상·prefix·next hop이 같고 interface가 현재 주 interface인 route 한정, 다른 interface의 route는 이 Helper가 그 interface로 넣은 것만 삭제
  5. 기록
     - 추가 전 routing table에서 아직 없는 route만 골라 그 route와 네트워크 서비스를 실행 기록 폴더에 기록 후 추가
     - Helper 재시작 시 앱 연결 수락 전 기록된 route 삭제(F-VR-06), 앱은 새 연결에서 재추가
  6. 종료
     - 앱 종료: 터널 종료 후 Helper에 이 앱의 route 삭제 요청, 응답 또는 정해진 시간까지 대기 후 종료
     - 앱 연결 끊김(비정상 종료): Helper가 그 앱만 원하던 route 삭제
     - macOS Helper의 유휴 종료 후: 삭제 요청으로 Helper 재시작, 새 Helper가 기록으로 삭제
  7. 실패: 쓰기 또는 Helper 연결 끊김 후 읽기의 연속 실패 시 정해진 횟수 재시도 후 macOS 알림 1회와 재시도 중단, 네트워크·규칙 변경 또는 준비 상태 확인의 Helper 응답 시 재개
  8. 이전 버전이 OS 설정에 저장한 예외 route: 읽기·삭제 없음, 설치 시 사용자가 직접 삭제

### F-BR-03 목록 편집 초안
- 설명: 예외 규칙 pane의 두 목록 편집 초안 관리, [적용] 시 저장
- 담당: 판단 코어, 런타임, UI
- 입력: 행 편집, [현재 네트워크 추가], [변경 취소], [적용]
- 출력: 행별 문제, 변경 여부
- 세부 기능
  1. 행별 확인: 형식 오류, 중복, 겹침, VPN 서버 주소(F-BR-01의 4), 문제가 있으면 [적용] 불가
  2. [변경 취소]: 저장된 목록으로 복귀, 같은 값의 다른 표기는 변경 아님
  3. 초안: 설정창을 닫아도 유지, 편집 중 밖의 목록 변경은 바뀐 항목만 병합

## 3.6 저장·기록 (F-SR)

### F-SR-01 state.json
- 설명: 사람이 편집할 필요 없는 기기별 상태 저장
- 담당: 런타임
- 입력: 저장할 상태
- 출력: state.json
- 세부 기능
  1. 저장 항목: 계정별 실패 기록, 로그인 시 열기 최초 등록 여부
     - 이전 버전의 예외 route 기록(`appliedExceptionRoutes`): 읽을 때 무시, 쓸 때 제외
  2. 변경 시에만 0600 권한의 원자적 쓰기, 읽기 실패 시 빈 상태로 시작
  3. 로그인 시 열기: 설치된 앱의 첫 실행 시 1회 켜기, 이후 사용자의 끄기 유지, Windows 앱 제거 시 모든 사용자의 등록 삭제(F-WN-04)

### F-SR-02 로그
- 설명: VPN client 원본 출력과 앱·Helper 설명 줄의 로그 pane·로그 파일 기록, 마스킹 없음
- 담당: 런타임, UI
- 입력: Helper 출력, 앱 설명 줄
- 출력: 로그 pane, 로그 파일
- 세부 기능
  1. Helper가 보낸 순서대로 줄 수집, 시각과 출처 추가
  2. 메모리에는 최근 줄만 유지, 파일은 0600 권한의 덧붙이기와 크기 초과 시 회전
  3. 앱 종료 전 파일 쓰기 완료, [지우기]는 화면만 비움
  4. [모두 복사]: UI가 화면에 가진 줄로 로그 파일과 같은 형식의 텍스트 생성
  5. 설명 줄: 영어, 비밀값과 config.yaml 내용 기록 금지
  6. Helper 실행마다 첫 줄에 VPN client 이름과 버전 기록
     - 버전 미확인 시 이름만
     - 시작 전 확인한 client 정보 사용, 이 줄을 위한 별도 실행 없음
  7. 시도마다 첫 줄 앞 빈 줄 두 개
     - 시도 시작 표시: 런타임 담당, 판단 코어가 시도를 시작하면 그 시도의 첫 줄에 표시
     - 포함: 시도를 시작하게 한 설명 줄, Helper가 실행 전에 거부한 시도의 줄
     - 인증서 재시도와 즉시 재연결: 각각 한 시도
     - 줄 없이 끝난 시도: 빈 줄 없음
     - 적용 대상: 로그 파일, [모두 복사], 로그 pane 동일
     - 앞에 줄이 없으면 빈 줄 생략: 새 파일·회전한 파일의 첫 줄, [모두 복사]의 처음, 로그 pane의 첫 줄([지우기]·오래된 줄 제거 후 포함)
  8. VPN client의 출력 상세도(verbosity): 기본값 고정

## 3.7 조율 (F-RT)

### F-RT-01 이벤트 처리와 타이머
- 설명: 앱의 모든 입력의 순차 판단과 정해진 순서의 실행, 같은 입력 순서에 항상 같은 결과
- 담당: 런타임, 판단 코어
- 입력: 모든 입력과 타이머
- 출력: Helper 요청, 파일 쓰기, port 요청, 구역 갱신
- 세부 기능
  1. 모든 입력의 단일 대기열 수신과 순차 판단, 할 일을 모두 실행한 뒤 다음 입력 처리, 실행 결과는 다시 입력
  2. Helper 요청은 정해진 순서로 하나씩 전송, Helper 메시지는 보낸 순서대로 처리
  3. 판단 코어가 정한 타이머를 런타임이 벽시계 시각으로 설정, 취소·변경된 타이머의 늦은 알림 무시
  4. 깨어남, 시계 변경, 시간대 변경 시 남은 타이머 재설정

### F-RT-02 UI 메시지
- 설명: 같은 프로세스 안 OS 앱 쪽(UI와 OS 연동)과 공통 부분 사이의 메시지 정의, macOS·Windows 공용
- 담당: 런타임, 공통 계약, UI, OS 연동
- 입력: 명령, port 응답, 시스템 이벤트
- 출력: 도메인 구역, 로그 추가분, port 요청
- 세부 기능
  1. UI의 사용자 조작 명령 전송(연결, 설정 저장, 예외 규칙 적용 등)
  2. 처음에는 모든 구역, 이후 바뀐 구역만 통째로 전송, 로그는 새 줄만 전송
  3. 시각은 절대 시각으로 전송, 카운트다운은 UI가 표시
  4. 오류는 문장이 아닌 종류와 값으로 전송, 바깥에서 온 설명(Google, IMAP 서버, Helper)은 그대로 포함
  5. 메시지 해석 실패 또는 상대의 예기치 않은 종료 시 즉시 중단

## 3.8 VPN 실행 (F-VR, Helper 쪽)

### F-VR-01 client 준비
- **설명**: 실행할 VPN client를 확인하고, 실행 요청을 검증해 실행 명세를 만든다.
- **담당**: backend adapter, process 모듈(Windows Helper가 시작할 때의 확인 실행도)
- **입력 / 출력**: 실행 요청 / client 경로와 버전, 실행 명세, 또는 오류
- **세부 기능**
  1. 관리자만 쓸 수 있는 곳의 실행 파일만 쓴다. 다른 사용자가 쓸 수 있거나 실행할 수 없으면 거부한다. 문제는 종류와 경로로 알리고, 문장과 번역은 UI가 만든다.
     - openfortivpn: `/opt/homebrew/bin/openfortivpn`을 끝까지 풀어 Homebrew Cellar 안의 파일만 쓴다.
     - openconnect: Windows는 앱과 함께 설치된 `C:\Program Files\FortiVPNManager\OpenConnect\openconnect.exe`만 쓴다. 따로 설치된 OpenConnect는 쓰지도 건드리지도 않는다. macOS·Linux는 지원할 때 정한다.
     - logout 요청 도구: OS에 들어 있는 것만 쓰고(Windows는 시스템 폴더), 그 도구의 사용자 설정 파일과 proxy 설정은 읽지 않게 한다.
  2. 검증된 버전(openfortivpn 1.24.x, openconnect 9.21)이 아니면 고급 pane에 "검증되지 않은 버전" 경고를 띄운다.
  3. openconnect의 버전은 실행해 물어서 알고, 상태 확인과 연결 때마다 다시 확인하며 기억하지 않는다. 이 확인에는 짧은 시간 제한을 둔다.
  4. Windows Helper의 시작 부분은 실행 중이라고 알리기 전에 process 모듈에 같은 확인을 한 번 맡긴다(openconnect `--version` 한 번). 이때는 시간 제한 없이 openconnect가 끝날 때까지 기다리고, 그동안 Windows에는 시작 중이라고 알린다(F-WN-03). 새로 설치한 client의 느린 첫 실행이 여기서 끝나므로 3의 짧은 제한으로 충분하고, 앱의 첫 상태 확인은 곧바로 답을 받는다. 이 결과도 기억하지 않는다.
  5. Helper는 앱을 믿지 않고 모든 값을 다시 검증한다. host는 IPv4나 host 이름만, 제어 문자 없음, 그 밖의 한도는 VPN client마다 정한다(openfortivpn은 버퍼 안, 비밀번호 255바이트 이하).
  6. 비밀번호와 openconnect의 세션 cookie는 설정 파일과 argv에 쓰지 않는다. openfortivpn 설정 파일은 0600으로 만들고 실행이 끝나면 지운다. openconnect는 설정 파일 없이 argv만 쓴다.
  7. verbosity 옵션과 자체 재접속(openfortivpn `--persistent`)을 쓰지 않는다. openconnect는 끊긴 뒤 스스로 다시 붙으려고 기다리지 않게 해서(대기 0초) 첫 재시도가 실패하면 곧바로 끝나게 한다. 재연결은 앱이 OTP와 함께 한다.
  8. 환경 변수는 꼭 필요한 것만 넘긴다. openfortivpn은 PATH만, openconnect는 그와 route 스크립트가 도는 데 필요한 OS 시스템 변수만 넘긴다.
  9. openconnect의 realm은 접속 URL의 경로로 넘기고, 인증서 고정 값은 두 실행 모두에 항상 하나 이상 넘긴다(F-CM-09).
  10. openconnect는 로그인만 하는 실행과, 그 세션 cookie로 터널을 여는 실행의 명세를 함께 만든다(F-VR-02). 세션 cookie로 보낼 logout 요청의 명세도 만든다(F-VR-04).
- **제약**: `--persistent`는 이메일 OTP 경로에서 쓴 OTP를 다시 보낸다. openfortivpn은 proxy 환경 변수를 읽고, 버퍼보다 긴 값은 조용히 자른다. openconnect의 비밀번호 미리 넣기 옵션은 쓰지 않는다. 인증서 질문이 그 줄을 답으로 읽는다. 비대화 옵션은 OTP 질문까지 막는다. 이 게이트웨이는 끊긴 세션으로 다시 붙는 것을 허용하지 않아서, openconnect가 다시 붙으려 하면 거부되고 logout한 뒤 끝난다. 다시 시도하는 동안에도 터널 어댑터는 남아 있어 앱은 끊긴 것을 알 수 없다. 연결하면서 cookie를 출력하는 openconnect 옵션은 정상 종료할 때까지 출력이 나오지 않아 쓰지 않는다.

### F-VR-02 실행과 stdin 입력
- **설명**: VPN client를 정해진 조건으로 띄우고, prompt가 나오면 비밀번호와 OTP를 stdin으로 한 번씩 넣는다.
- **담당**: process 모듈, backend adapter
- **입력 / 출력**: 실행 명세, 비밀번호, OTP / 진행 단계, 입력 결과
- **세부 기능**
  1. shell 없이 실행한다. 자식은 pipe 세 개만 물려받는다. Unix에서는 빈 signal mask와 기본 signal 동작으로 자기 process group에서 시작한다. Windows에서는 창 없이 시작한다.
  2. prompt는 줄 끝을 기다리지 않고 찾는다. 비밀번호는 backend adapter가 정한 단계의 첫 prompt에만 쓴다(openfortivpn은 게이트웨이 연결 전, openconnect는 OTP prompt 전). openconnect는 게이트웨이 인증서를 확인한 뒤에만 묻는다. 연결 줄은 stdout, prompt는 stderr로 따로 읽혀 순서가 바뀔 수 있으므로 연결 줄을 기다리지 않는다.
  3. OTP는 OTP prompt 단계에서 한 번만 받고, 쓴 뒤 stdin을 닫는다.
  4. 비밀번호나 OTP를 다시 물으면 답하지 않고 바로 끝낸다. 비밀번호를 다시 물으면 로그인 거부, OTP를 다시 물으면 OTP 거부다. 시간 제한까지 기다리지 않기 위해서다.
  5. 쓴 비밀값은 바로 메모리에서 지운다. client가 먼저 끝나도 Helper는 죽지 않는다.
  6. openconnect는 두 번 실행한다. 첫 실행은 비밀번호와 OTP로 로그인만 하고 세션 cookie를 출력한 뒤 끝난다. 그 cookie를 stdin으로 받는 둘째 실행이 터널을 연다. 앱에는 한 번의 시도로 보이고, 진행 단계는 두 실행에 걸쳐 이어서 보고한다. 첫 실행이 cookie 없이 끝나면 둘째 실행 없이 그 종료를 보고한다.
  7. 세션 cookie는 Helper 메모리에만 두고, 실행이 끝나면 지운다. 로그에는 값 대신 받았다는 사실만 남긴다.
- **제약**: prompt에는 줄바꿈이 없다. signal mask를 비워야 openfortivpn이 터널 전 SIGTERM에 즉시 끝난다. openconnect는 로그인이 거부되면 끝나지 않고 다시 묻는다.

### F-VR-03 출력 해석
- **설명**: client 출력을 로그 pane의 줄로 만들고 진행 단계, 인증서 값, 실패 단서, 터널 IP를 읽는다. 출력 문구는 backend adapter만 안다.
- **담당**: backend adapter, process 모듈
- **입력 / 출력**: stdout·stderr / 출력 줄, 진행 단계, 인증서 값, 실패 단서, 터널 IP
- **세부 기능**
  1. 줄바꿈 없는 출력(prompt)은 잠시 뒤 따로 보이고, 제어 문자는 보이는 표기로 바꾼다. UTF-8이 아닌 바이트도 줄을 버리지 않고 보이는 표기로 바꾼다.
  2. openfortivpn은 로그 머리가 붙은 stdout 줄만 해석한다. `Connected to gateway.`, `Authenticated.`, `Tunnel is up and running.`, `trusted-cert = …`, tunnel mode 거부 줄이 대상이다.
  3. openconnect는 stdout과 stderr를 모두 해석한다. 대상은 다음과 같다.
     - 게이트웨이 연결: `Connected to HTTPS on …`
     - OTP prompt: `Code:`
     - 인증됨: 첫 실행의 세션 cookie 줄(`COOKIE='…'`), 둘째 실행의 `Got Legacy IP address …`
     - 터널 IP: `Configured as …`
     - 터널 올라옴: 터널 어댑터를 만들었다는 줄(Windows는 `Using Wintun device …`. 다른 OS는 지원할 때 정한다)
     - 인증서 값: `None of the … fingerprint(s) … match server's certificate: …`
  4. 단계는 한 번씩, 커지는 순서로만 보고한다.
  5. openfortivpn이 route 복원을 끝내지 못하고 끝났는지 추적한다.
- **제약**: openfortivpn의 stderr에는 pppd 메시지가 나오며 오류를 뜻하지 않는다. openconnect는 prompt와 오류를 stderr에 쓴다. `Configured as` 줄은 터널 어댑터가 생기기 전에 나오므로 터널 올라옴으로 보지 않는다. 그 뒤 route 스크립트가 route를 다 넣기까지 몇 초가 더 걸리지만 기다리지 않는다(macOS에서도 route를 확인하지 않는다). route 스크립트가 openconnect의 10초 제한을 넘기면 나오는 "스크립트 실패" 줄은 실패로 보지 않는다. openconnect도 무시하고 터널을 유지한다. 날짜와 Windows 오류 문장 같은 일부 문구는 Windows 언어 설정에 따라 번역되고 UTF-8이 아니다. VPN client를 올리면 문구가 바뀔 수 있다.

### F-VR-04 시간 제한과 종료
- **설명**: 단계마다 시간 제한을 두어 진행하지 못하는 client를 끝내고, 요청이 오면 정해진 순서로 종료시킨다.
- **담당**: process 모듈, backend adapter
- **입력 / 출력**: 진행 단계, 종료 요청 / 정상 종료 요청과 강제 종료, 종료한 주체
- **세부 기능**
  1. 터널이 올라오기 전까지 단계마다 시간 제한을 둔다. 넘으면 늦은 구간을 로그에 남기고 종료한다.
  2. 먼저 정상 종료를 요청하고, 유예 뒤에도 살아 있으면 강제로 끝낸다.
     - Unix: SIGTERM은 본 프로세스에만 보낸다. 자식 `/sbin/route`가 route 복원을 끝내야 하기 때문이다. 유예 뒤에는 SIGKILL을 본 프로세스, 그다음 process group에 보낸다.
     - Windows: 정상 종료를 요청할 수단이 없어 곧바로 강제로 끝낸다. client와 그것이 띄운 프로세스(route 스크립트)를 함께 끝낸다.
  3. Helper가 openconnect의 세션 cookie를 가진 채 openconnect를 강제로 끝냈으면, 게이트웨이에 그 cookie로 logout 요청을 보낸다. 정상 종료 요청으로 끝났거나 스스로 끝났으면 openconnect가 logout했으므로 보내지 않는다. 응답으로는 성공 여부를 알 수 없어 보낸 사실과 응답 코드만 로그에 남긴다. 기다리는 시간에는 상한을 둔다.
  4. 첫 종료 이유만 기록하고 이후 요청은 무시한다.
- **제약**: openfortivpn의 SIGTERM handler는 터널이 올라온 뒤에만 설치된다. Windows에는 signal이 없고, openconnect가 logout하는 console 이벤트는 그 이벤트를 보내는 실행 파일을 회사 보안 프로그램이 막아 쓸 수 없다. 강제로 끝내면 openconnect가 logout하지 못해 세션이 idle timeout까지 서버에 남지만, 따로 보낸 logout 요청으로 끝난다. Helper가 죽어 cookie를 잃으면 세션이 남는다. 이 게이트웨이는 같은 계정의 동시 세션을 허용하므로 다음 연결은 막히지 않는다.

### F-VR-05 종료 사실과 보고
- **설명**: client가 끝나면 무슨 일이 있었는지를 종료 사실로 앱에 한 번 보고한다. 앱은 이것으로 원인을 분류한다(F-CM-03).
- **담당**: process 모듈, entry 모듈
- **입력 / 출력**: 프로세스 종료 / 종료 사실
- **세부 기능**
  1. 가장 멀리 간 단계, 종료한 주체와 이유, 학습한 인증서 값, 실패 단서, 터널 유지 시간, 종료 상태를 담는다.
  2. logout 요청(F-VR-04)을 보낸 뒤, route가 정리되지 않았으면 정리하고 보고한다. openfortivpn은 default route를, openconnect는 게이트웨이로 가는 host route를 본다(강제로 끝나면 남는다).
  3. 보고는 실행마다 정확히 한 번이고 마지막 메시지다.

### F-VR-06 남은 프로세스 정리와 route 복구
- **설명**: 비정상 종료 뒤 남은 VPN client 프로세스와 깨진 route, 이전 Helper가 넣은 예외 route를 정리한다. openfortivpn이 비정상 종료하면 default route가 사라지고, openconnect가 강제로 끝나면 게이트웨이 host route가 남기 때문이다.
- **담당**: recovery 모듈, entry 모듈, Helper OS 영역
- **입력 / 출력**: pidfile의 프로세스 신원과 게이트웨이 IP, 예외 route 기록 / 정리 결과, route 복구 결과
- **세부 기능**
  1. 실행 중에는 client(openfortivpn은 pppd도)의 신원과 게이트웨이 IPv4를 pidfile에 기록한다.
  2. Helper가 시작하면 연결을 받기 전에, 기록과 같은 관리자 권한 프로세스인 VPN client만 종료한다. 다른 프로세스는 건드리지 않는다. Windows에서는 Helper 서비스가 죽어도 openconnect가 혼자 남으므로 이 정리가 필요하다.
  3. macOS: default route가 없거나 ppp를 가리키면 주 네트워크의 router로 되돌리고, routing table을 다시 조회해 확인한다.
  4. Windows: 남은 openconnect가 없는데 router를 거쳐 게이트웨이로 가는 host route가 남아 있으면 지우고 다시 조회해 확인한다. 강제로 끝난 터널 어댑터는 다음 openconnect 실행이 스스로 지우므로 건드리지 않는다.
  5. 이전 Helper가 기록해 둔 예외 route를 연결을 받기 전에 지운다. 지우지 못한 route는 기록에 남기고 이 Helper가 넣은 것으로 다뤄, 어느 앱도 원하지 않으면 다음 변경 때 다시 지운다(F-BR-02).
- **제약**: `/sbin/route`는 실패해도 exit code 0이다. pid는 재사용될 수 있어 시작 시각과 경로로 같은 프로세스인지 확인한다.

### F-VR-07 Helper 요청 처리
- **설명**: 앱의 요청으로 실행을 시작·중지하고 Helper 자신의 수명을 관리한다. 실행은 한 번에 하나다.
- **담당**: entry 모듈과 Helper OS 영역
- **입력 / 출력**: 앱 요청, 앱 연결 끊김, 재설치, SIGTERM·서비스 정지 / 응답, 이벤트와 출력
- **세부 기능**
  1. 실행이 있으면 새 실행 요청은 같은 사용자에게 "사용 중", 다른 사용자에게 "권한 없음"이다. Windows는 Windows 세션으로 구분하고, 이 PC의 어느 계정이든 연결할 수 있다.
  2. OTP와 연결 해제는 현재 실행의 시도이고 같은 사용자일 때만 받는다.
  3. 앱 연결이 끊기면 그 앱의 실행을 끝내고, 그 앱만 원하던 예외 route를 지운다(F-BR-02). 정상 종료, 강제 종료, 크래시가 같은 경로다.
  4. macOS: 유휴 시 종료 요청에는 실행이 없을 때만 응하고 종료한다. 버전 확인과 이 요청의 모양은 빌드가 바뀌어도 바꾸지 않는다.
  5. macOS: 재설치로 실행 파일이 바뀌면 실행이 없을 때 종료하고, launchd가 다음 요청 때 새 Helper를 띄운다.
  6. SIGTERM이나 서비스 정지 요청을 받으면 실행을 끝내고 보고한 뒤 종료한다. Windows의 업그레이드와 제거는 앱을 닫고 서비스를 멈춘 뒤 파일을 바꾸거나 지운다. 업그레이드 뒤 새 Helper는 앱이 시작한다(F-WN-05).
- **제약**: 실행 중인 옛 Helper는 재설치 뒤 앱의 서명 확인을 통과하지 못한다. 이 자가 종료 덕분에 `make install`에 sudo가 필요 없다. Windows에서는 실행 중인 실행 파일을 바꿀 수 없어 설치 패키지가 서비스를 먼저 멈춘다.

## 3.9 macOS 부분 (F-SW)

### F-SW-01 UI 표시와 문장
- **설명**: 도메인 구역을 메뉴바와 설정창으로 그리고 조작을 명령으로 보낸다.
- **담당**: UI
- **입력 / 출력**: 도메인 구역, 로그 추가분 / 화면, 명령
- **세부 기능**
  1. 메뉴바 아이콘과 메뉴, 일반·계정·연결 관리·예외 규칙·고급·로그 pane의 설정창을 그린다. 설정창은 시스템 설정처럼 사이드바와 설정 검색으로 pane을 고른다. 인증서 항목은 없다.
  2. 버튼 상태와 값은 구역 값 그대로다. 다만 메뉴와 일반 pane의 [연결] [연결 해제] [지금 재연결]은 늘 보이고, 숨김 값이면 흐리게 그린다.
  3. 문장 조립, 그리고 설정 필요 항목마다 둔 동작과 실패 안내 버튼이 열 pane은 UI가 정한다.
  4. Rust가 보낸 종류와 값으로 지금과 같은 문장을 만들고 String Catalog로 번역한다. "헬퍼"가 아니라 "Helper"라고 쓴다.
  5. 로그 pane 색은 출처가 아니라 `ERROR:`·`WARN:` 머리로 정한다. stderr는 대부분 평범한 pppd 메시지다.
- **제약**: main actor에서 modal 창을 열면 main queue가 멈춰 Rust 메시지 처리도 멈춘다. 파일 선택은 sheet로 연다.

### F-SW-02 OS port
- **설명**: Rust가 요청하는 OS 기능을 macOS API로 구현하고 시스템 이벤트를 알린다.
- **담당**: OS 연동
- **입력 / 출력**: port 요청 / port 응답, 시스템 이벤트

| port | macOS 구현 |
|---|---|
| 비밀값 저장 | 로그인 Keychain(서비스 이름과 계정 키는 지금과 같음) |
| 권한 있는 실행 통로 | SMAppService daemon(등록, 상태, 승인 설정 열기), 양쪽 서명을 확인하는 XPC |
| 시스템 이벤트 | NSWorkspace(잠자기, 깨어남, 디스플레이 잠자기), `com.apple.screenIsLocked`/`Unlocked`, NWPathMonitor, SCDynamicStore(네트워크 설정 변경), 앱 활성화, 시계·시간대 변경 알림 |
| 화면 상태 | CGSession의 잠금 상태, 디스플레이 잠자기 여부 |
| 네트워크 정보 | `/sbin/route -n get`, getifaddrs, SCDynamicStore |
| 알림 | UserNotifications |
| 브라우저 열기 | 기본 브라우저 |
| 앱 수명 | ProcessInfo activity, 종료 확인, SMAppService main app(로그인 시 열기) |
| 로그 출력 | 통합 로그 |

- **세부 기능**
  1. 비밀번호나 route를 바꾸는 Helper 요청은 같은 빌드로 확인된 연결로만 보낸다.
  2. Helper 메시지는 받은 순서대로 넘긴다.
  3. 사실만 알리고, 자리 비움 같은 판단은 Rust가 한다.
- **제약**: XPC callback과 Rust 메시지 읽기 callback은 `@Sendable`로 선언한다. 앱의 기본 격리가 MainActor라서 아니면 크래시한다.

### F-SW-03 Helper OS 영역 (macOS)
- **설명**: Helper 프로세스에서 OS에 의존하는 부분이다. 나머지는 entry 모듈에 넘긴다.
- **담당**: Helper OS 영역 (macOS)
- **입력 / 출력**: XPC 연결과 요청, SIGTERM / entry로 전달, XPC 응답과 이벤트
- **세부 기능**
  1. 남은 프로세스와 route의 정리(F-VR-06)가 끝났다고 entry가 알린 뒤에 XPC 연결을 받는다.
  2. 앱의 서명(Team ID + signing identifier)을 통과한 연결만 받고 버전 확인에 직접 답한다. 다른 요청은 내용을 보지 않고 entry에 넘긴다.
  3. 자기 실행 파일 교체를 감지해 entry에 알린다.
  4. route 복구, entry가 요청한 예외 route 넣기·지우기와 주 네트워크 조회를 `/sbin/route`와 SCDynamicStore로 한다. 넣고 지운 뒤에는 routing table을 다시 조회해 확인한다. 터널 제어가 이를 기다리지 않게 따로 처리한다.
  5. 시스템 명령은 고정 argv, PATH만, shell 없이, 시간 제한을 두고 실행한다.
- **제약**: Personal Team 서명이라 entitlement는 비워 두고, 번들 ID는 바꾸지 않는다.

## 3.10 Windows 부분 (F-WN)

### F-WN-01 UI 표시와 문장 (Windows)
- **설명**: 도메인 구역을 알림 영역 아이콘과 설정창으로 그리고 조작을 명령으로 보낸다. 규칙은 F-SW-01과 같고, 다른 점만 적는다.
- **담당**: UI (Windows)
- **입력 / 출력**: 도메인 구역, 로그 추가분 / 화면, 명령
- **세부 기능**
  1. 알림 영역 아이콘은 앱 아이콘이고, 연결 상태는 툴팁과 메뉴로 알린다. Windows 알림 영역 아이콘은 macOS처럼 상태별 기호로 그리는 방식이 아니어서다(사용자 결정, 2026-11-13). 아이콘을 누르면 macOS 메뉴와 같은 항목의 메뉴(Windows 기본 메뉴)가 열린다.
  2. 설정창은 macOS와 같은 pane을 같은 순서로 두고, macOS처럼 창 왼쪽의 사이드바로 고른다. pane 안의 항목도 macOS와 같은 묶음과 순서로 둔다.
  3. 설정창을 닫아도 앱은 알림 영역에 남는다. 앱을 끝내는 것은 메뉴의 [종료]뿐이다. 닫은 설정창은 창이 쓰던 메모리를 돌려주도록 숨기지 않고 없애고, 다시 열 때 새로 만든다. 창의 자리와 크기, 마지막으로 본 pane은 이어진다(사용자 결정, 2026-11-14). [종료]를 누르면 알림 영역 아이콘과 설정창이 바로 사라지고, 연결을 끝내는 일(로그아웃, route 정리)은 보이지 않게 이어진다. 그동안 앱을 다시 실행하면 이전 앱이 끝나기를 기다렸다가 새로 시작한다(사용자 결정, 2026-11-15).
  4. 앱을 켜면 Helper 서비스가 실행 중이 될 때까지 아무것도 보이지 않고(F-WN-05), 그 뒤 알림 영역 아이콘만 둔다. 설정이 필요하거나 첫 실행에서 설정을 가져왔어도 설정창을 스스로 열지 않는다. 설정창은 아이콘 더블클릭이나 메뉴의 [설정…]으로 연다(사용자 결정, 2026-11-13).
  5. 문장은 macOS와 같은 뜻으로 만들고, Windows 표시 언어(한국어·영어)를 따른다.
  6. Helper 승인 관련 항목은 없다. 일반 pane의 설정 가져오기는 openfortivpn 대신 FortiClient 연결에서 가져온다(F-CF-04).
  7. OpenConnect는 앱과 함께 설치되므로, OpenConnect를 찾을 수 없거나 쓸 수 없으면 OpenConnect를 따로 설치하라고 하지 않고 FortiVPNManager를 다시 설치하라고 안내한다.

### F-WN-02 OS port (Windows)
- **설명**: Rust가 요청하는 OS 기능을 Windows에서 구현하고 시스템 이벤트를 알린다. port의 종류와 메시지는 macOS와 같다.
- **담당**: OS 연동 (Windows)
- **입력 / 출력**: port 요청 / port 응답, 시스템 이벤트

| port | Windows 구현 |
|---|---|
| 비밀값 저장 | Credential Manager의 사용자 자격 증명. 항목 이름은 macOS Keychain의 서비스 이름과 계정 키를 따른다 |
| 권한 있는 실행 통로 | Helper 서비스의 설치·실행 상태와 시작(F-WN-05), 이 PC의 계정이 여는 연결 통로. 서비스 등록은 설치 패키지가 한다(F-WN-04) |
| 시스템 이벤트 | 절전과 재개, 세션 잠금과 해제, 디스플레이 꺼짐과 켜짐, 네트워크 주소·연결 변경, 앱 활성화, 시계·시간대 변경 |
| 화면 상태 | 세션 잠금 여부, 디스플레이 꺼짐 여부 |
| 네트워크 정보 | interface 목록(주소, gateway, interface GUID), 게이트웨이로 가는 경로 |
| 알림 | Windows 알림 |
| 브라우저 열기 | 기본 브라우저 |
| 앱 수명 | 종료 확인, 로그인 시 열기(사용자 시작 프로그램). 활동 유지는 필요 없다 |
| 로그 출력 | Windows 이벤트 로그 |
| FortiClient 연결 | 컴퓨터 설정(`HKLM\SOFTWARE\Fortinet\FortiClient\Sslvpn\Tunnels`)의 연결별 key 이름과 `Server` 값. 읽기만 한다. macOS는 빈 목록으로 답한다 |

- **세부 기능**
  1. Helper 통로를 열면 그 소유자가 시스템 계정인지 확인한다. 같은 이름의 통로를 다른 프로그램이 먼저 만들어 비밀번호를 가로채는 일을 막는다.
  2. 비밀번호나 route를 바꾸는 Helper 요청은 같은 빌드로 확인된 연결로만 보낸다.
  3. Helper 메시지는 받은 순서대로 넘긴다.
  4. 사실만 알리고, 자리 비움 같은 판단은 Rust가 한다.
  5. 시간대는 IANA 이름으로 바꿔 알린다. 판단 코어가 IANA 시간대로 계산하기 때문이다.
  6. interface는 Windows가 보여 주는 어댑터 이름으로 부른다. 경로, 주소, 네트워크 변경 이벤트가 같은 이름을 써야 런타임이 맞춰 볼 수 있다.
  7. route가 사라져도 알림이 없으므로, 주소가 바뀔 때와 5분마다 네트워크 설정이 바뀌었다고 알린다(F-BR-02).
  8. Windows에서 로그아웃하거나 끌 때는 메뉴의 [종료]와 같이 종료를 요청한다. 설치 패키지가 업그레이드나 제거 때 앱을 닫으려고 보내는 요청도 같다(F-WN-04).

### F-WN-03 Helper OS 영역 (Windows)
- **설명**: Windows의 Helper OS 영역이다. 서비스(Helper 서비스)로 실행되며 entry, process, recovery 모듈을 쓰고, macOS의 Helper OS 영역이 하는 일을 함께 한다.
- **담당**: Helper OS 영역 (Windows)
- **입력 / 출력**: 서비스 시작·정지, 앱 연결과 요청 / 응답, 이벤트와 출력, route 변경
- **세부 기능**
  1. 설치 패키지가 등록하고(F-WN-04), 부팅 때 자동으로 시작하고, 비정상 종료하거나 시작에 실패하면 Windows가 다시 시작한다. 설치 직후와 업그레이드 뒤처럼 실행 중이 아니면 앱이 시작한다(F-WN-05). Windows가 꺼질 때는 다른 서비스와 네트워크가 내려가기 전에 오는 종료 준비 알림에서 실행을 끝낸다. 그 뒤의 종료 알림은 몇 초만 기다려 주어 logout이 잘리기 때문이다.
  2. 남은 프로세스와 route를 정리한 뒤에(F-VR-06) 앱 연결을 받는다.
  3. Windows에 실행 중이라고 알리기 전에, 시작 부분이 process 모듈에 client 확인을 한 번 맡겨 VPN client를 실행해 본다(F-VR-01의 4). 시간 제한 없이 client가 끝날 때까지 기다리고, 그동안 Windows에는 시작 중이라고 알린다. 새로 설치한 client의 느린 첫 실행이 서비스가 시작하는 동안에 끝나므로, 앱의 첫 상태 확인은 곧바로 답을 받는다. 결과는 기억하지 않는다. 상태 확인과 연결은 여전히 매번 짧은 제한으로 확인한다.
  4. 연결 통로는 이 PC의 계정과 시스템이 열 수 있고(다른 컴퓨터의 연결은 받지 않는다), 시스템 계정이 소유한다. 버전 확인에 직접 답하고, 앱이 함께 보낸 빌드가 같은 연결에서만 연결·OTP·예외 route 요청을 처리한다.
  5. 유휴 시 종료 요청에는 끝나지 않는다고 답한다. Windows는 끝난 서비스를 다시 띄우지 않고, 업그레이드는 앱과 Helper를 함께 바꾼 뒤 다시 연 앱이 새 Helper를 시작하기 때문이다.
  6. VPN client를 끝낼 때는 그것이 띄운 프로세스까지 함께 끝낸다(F-VR-04). 남은 openconnect를 정리할 때도 같다(F-VR-06). 둘 다 process 모듈의 Windows 부분이 한다.
  7. entry가 요청한 예외 route 넣기·지우기와 주 interface 조회, 게이트웨이 host route 정리를 Windows API(IP Helper)로 직접 하고, 바꾼 뒤 다시 조회해 확인한다. 터널 제어가 이를 기다리지 않게 따로 처리한다. 넣기 전에 주 interface가 앱이 읽은 그대로인지 확인하고, 그 interface의 gateway로 가는 올바른 예외 대상만 넣는다.
  8. 시스템 명령은 고정 argv, 필요한 환경 변수만, shell 없이, 시간 제한을 두고 실행한다. 번역되는 출력은 해석하지 않는다.
  9. 실행 기록은 시스템과 관리자만 접근할 수 있는 곳에 둔다.
  10. 로그는 Windows 이벤트 로그에 쓴다. 로그 출처는 설치 패키지가 등록한다(F-WN-04).
- **제약**: 연결한 프로그램이 설치된 앱인지는 확인하지 않는다. 같은 사용자로 실행되는 다른 프로그램도 연결할 수 있다(DESIGN §8). 그래서 한 메시지의 크기와, 읽지 않은 채 쌓이는 보낼 메시지의 수에 상한을 두고, 예외 route 요청은 연결마다 한 번에 하나만 받는다. 보낼 메시지가 넘친 연결은 앱이 끝난 것처럼 다룬다. 서비스에는 화면이 없어서, 사용자에게 알릴 일은 모두 앱으로 보낸다.

### F-WN-04 설치, 업그레이드, 제거 (Windows)
- **설명**: 앱, Helper, OpenConnect를 MSI 하나로 설치하고 바꾸고 지운다. 관리자가 배포하든 사용자가 직접 설치하든 결과가 같다(사용자 결정, 2026-11-13).
- **담당**: 설치 패키지 (Windows)
- **입력 / 출력**: MSI 실행(화면 또는 quiet), 설정 > 앱의 수정·제거 / 설치된 파일과 등록, 업그레이드, 제거 때의 정리
- **세부 기능**
  1. Windows 11 이상 x64에 관리자 권한으로 이 PC 전체에 설치한다(per-machine). 폴더는 `C:\Program Files\FortiVPNManager`로 고정하고 아무것도 묻지 않는다. quiet 설치(`msiexec /qn`)와 화면 설치의 결과는 같다.
  2. 화면은 Welcome(설치하는 동안 VPN 연결이 끊긴다는 안내) → Progress → Finish뿐이다. license, 폴더, 기능 고르기 화면은 없다. 화면 언어는 Windows 표시 언어가 한국어면 한국어, 그 밖에는 영어다.
  3. 앱(실행에 필요한 .NET runtime과 Windows App SDK 부분 포함), Helper, OpenConnect 9.21(앱 폴더 아래 `OpenConnect`), OpenConnect와 그 안에 든 라이브러리들의 license 문서를 설치한다. PC에 따로 설치하는 것은 없다. 앱이 쓰지 않는 Windows App SDK 구성 요소(AI, ML 등), 한국어와 영어 밖의 WinUI 언어 리소스, 디버그 심볼은 넣지 않는다.
  4. Helper 서비스를 SYSTEM 계정으로 등록한다. 부팅 때 자동으로 시작하고, 비정상 종료하거나 시작에 실패하면 Windows가 다시 시작하며, Windows가 꺼질 때는 종료 준비 알림 뒤 Helper를 기다려 준다. 서비스 권한은 Windows의 기본값(일반 계정은 상태와 설정을 볼 수 있다)에 모든 계정의 시작만 더한다. 멈추기, 설정 바꾸기, 지우기는 관리자만 한다. 설치는 서비스를 시작하지 않는다. 앱이 시작한다(F-WN-05).
  5. 앱과 Helper의 이벤트 로그 출처와 모든 사용자의 시작 메뉴 바로 가기를 등록한다.
  6. 설정 > 앱에는 수정(Repair, Remove)과 제거가 있다. 빠진 파일을 Repair로 되살리려면 처음 설치한 .msi가 필요하다(Windows가 그 위치를 기억한다).
  7. 새 MSI는 이전 버전을 바꾼다. 이전 버전은 같은 transaction 안에서 지우므로 설치가 실패하면 이전 버전이 돌아온다. 더 낮은 버전은 설치하지 않는다.
  8. 업그레이드와 제거는 실행 중인 앱을 닫는다. Windows 로그아웃 때처럼 종료를 요청하고(F-WN-02), 몇 초 뒤에도 남아 있으면 끝낸다. "사용 중인 파일" 창과 다시 시작 요청은 없다. 업그레이드 뒤 앱은 사용자가 다시 열 때까지 닫혀 있다. 로그인 시 열기가 켜져 있으면 다음 로그인 때 열린다.
  9. 제거는 실제 제거일 때만 다음을 한다. 업그레이드 중 이전 버전을 지울 때는 하지 않는다.
     - 앱을 닫고, Helper를 멈추고 서비스를 지운다. 앱이 끝나면서 예외 route를 지운다(F-BR-02).
     - 모든 사용자의 로그인 시 열기 등록을 지운다(F-SR-01).
     - Helper의 실행 기록 폴더, 설치한 파일, 시작 메뉴 바로 가기, 두 이벤트 로그 출처를 지운다.
  10. 사용자마다의 config.yaml, state.json, 로그, 저장된 비밀값은 남긴다. 로그인 시 열기 등록을 읽을 수 없는 사용자는 로그에 남기고 건너뛴다. 그 때문에 제거가 실패하지는 않는다.
- **제약**: 설치 패키지는 다른 설치 프로그램을 실행할 수 없어서 필요한 것을 모두 담는다. 화면 언어는 문서화되지 않은 Windows Installer 동작에 기대므로, 동작하지 않게 되면 영어 화면으로 설치된다(DESIGN §9).

### F-WN-05 앱 시작과 Helper 실행 (Windows)
- **설명**: 앱이 맨 먼저 Helper 서비스가 실행 중이게 한다. Helper 없이는 앱이 쓸모가 없고, 다른 사람은 느린 첫 시작과 앱의 고장을 구분할 수 없으므로 그동안 보이지 않게 기다린다. 실행하지 못하면 이유를 알리고 끝난다(사용자 결정, 2026-11-13·14).
- **담당**: OS 연동 (Windows), UI (Windows)
- **입력 / 출력**: Helper 서비스 상태 / 서비스 시작, 알림 영역 아이콘 표시, 또는 오류 창과 앱 종료
- **세부 기능**
  1. 앱을 켜면 다른 무엇을 보이거나 시작하기 전에 이 일을 끝낸다. 그동안 알림 영역 아이콘도 없다.
  2. 서비스가 실행 중이면 곧바로 넘어간다. 시작하는 중이면 기다리고, 멈춰 있으면 시작한 뒤 기다린다. 서비스가 시작 중이라고 알리는 동안에는 얼마가 걸리든 기다린다. 이 흐름에는 시간 제한이 없다.
  3. 준비는 서비스가 실행 중이라는 것뿐이다. Helper의 첫 답을 따로 기다리지 않고, Helper와 주고받는 응답 시간 제한은 평소와 같다. Helper는 실행 중이라고 알리기 전에 VPN client 확인을 마치므로(F-WN-03) 첫 상태 확인은 곧바로 답을 받는다.
  4. 실패한 시작은 macOS 앱의 "한 번 더 시도" 규칙처럼 한 번만 다시 한다.
     - Windows가 기다리기를 그만둬 시작이 실패했으면, 그 시도의 프로세스가 끝날 때까지 기다린 뒤 다시 시작한다.
     - Helper가 시작하다 오류로 멈췄으면 곧바로 다시 시작한다.
     - 첫 실패는 보이지 않고 로그에만 남긴다(시도 1/2회).
  5. 서비스가 설치돼 있지 않거나, 사용 안 함이거나, 시작할 권한이 없으면 곧바로 오류 창을 띄운다. 첫 줄은 "Helper 실행을 실패하여 FortiVPNManager를 종료합니다."이고, 그 아래에 이유와 "FortiVPNManager를 다시 설치하거나 관리자에게 문의하세요."를 둔다.
  6. 다시 한 시작도 실패하면 오류 창의 첫 줄은 "Helper 시작 도중 오류가 발생하였습니다. FortiVPNManager를 종료합니다. (시도 2/2회)"이고, 그 아래에 이유("Helper가 제때 시작되지 않았습니다" 또는 "Helper가 시작 중 오류로 멈췄습니다")와 5와 같은 안내를 둔다.
  7. 오류 창을 닫으면 앱이 끝난다. 영어 화면에서는 같은 뜻의 영어 문장이다.
  8. 서비스가 실행 중이 되면 알림 영역 아이콘을 둔다(F-WN-01).
- **제약**: 서비스는 SYSTEM 계정으로 돌고 설치 패키지는 서비스를 시작하지 않으므로, 이 PC의 모든 계정에 시작 권한이 있어야 한다(F-WN-04). macOS에는 이 단계가 없다. macOS는 사용자가 Helper의 권한을 직접 승인해야 하므로 앱이 먼저 떠서 그 과정을 안내해야 하지만, Windows는 설치 패키지가 서비스를 등록하므로 Helper가 실행되기 전에는 앱이 보일 것이 없다.

# 4. 비기능 요구사항

- **비밀값**: 비밀번호, OTP, token, client secret은 argv, 파일, 환경 변수, 로그에 남지 않는다. Helper에는 앱과 Helper 사이의 연결로 메모리 안에서만 전달한다.
- **fail fast**: Rust panic은 그 자리에서 중단하고 crash report를 남긴다. 있을 수 없는 상태를 위한 복구 경로를 만들지 않는다.
- **unsafe 없음**: Rust에 `unsafe` 블록을 두지 않는다. 예외는 시작 함수 표시(crate마다 하나만)와 Windows Helper 서비스의 os_api 모듈이다. os_api는 Helper가 쓰는 Windows API를 safe 함수로 감싸고, 다른 코드는 Windows API를 직접 부르지 않고 이 함수를 쓴다.
- **OS 앱 쪽과 Rust**: 메시지만 주고받고 서로에게 포인터를 넘기지 않는다. OS 앱 쪽 코드(Swift, C#)가 OS API를 직접 부를 때는 그 API에 필요한 포인터 API나 호출 선언을 쓸 수 있다.
- **다른 OS 빌드**: 공통 crate는 한 OS 전용 API 없이 다른 OS에서도 빌드된다. OS마다 다른 것은 process 모듈의 OS 부분(프로세스 실행·확인·종료, 실행 기록 폴더의 접근 권한), Helper OS 영역(시작 부분 포함), port 구현뿐이다.
- **영어**: 코드, 주석, 앱과 Helper의 로그는 영어로 쓴다.
- **화면 문장은 UI에만**: Rust는 종류와 값만 넘기고, 문장과 번역은 UI (macOS)와 UI (Windows)가 만든다.
