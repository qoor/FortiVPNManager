---
paths:
  - "components/**"
  - "packaging/**"
  - "docs/SPEC.md"
---

# 연결 관리
- 같은 계정의 동시 세션 허용
- 회사 VPN 서버: 분할 터널 한정
  - 전체 터널의 연결됨 확인 실측 불가
  - Windows 전체 터널 판정(0.0.0.0의 GetBestRoute2) 미검증

# OTP
- OTP 유효시간 약 60초
- 메일 도착 후 감지까지 걸린 시간
  - IMAP: 약 7초
  - Gmail API: 3~12초, 중앙값 약 4.6초

# openconnect
- 끊김 감지
  - 어댑터가 꺼지면 즉시 감지
  - 게이트웨이와의 통신만 끊기면 약 20초 뒤 감지
  - 재시도 중에도 터널 어댑터는 Up 상태 유지
  - 끊김 확인: 콜백·종료 코드 기준
- 감지 뒤 내부 재접속
  - 실측(macOS): 통신이 끊긴 동안 TCP 연결 약 75초 대기 후 실패
  - 회사 VPN 서버: 끊긴 세션의 재접속 거부
    - 서버 응답: "reconnect-after-drop is not allowed"
  - 결과: 세션 종료 후 앱의 새 로그인으로 복구
- logout
  - logout 요청의 응답은 가짜 cookie에도 같아 성공 여부 판단 불가
  - 세션 정보 요청 응답으로 판단 가능: 터널이 열린 세션 403, 끝난 세션 302
  - 시간 제한 부재
    - 통신이 끊긴 동안 계속 대기
    - 실측(macOS): 통신 복구 후 약 24초 뒤 완료
- 라이브러리
  - 스크립트 없이 연 터널 장치 미닫힘
  - 본문 잘림 + TLS 종료 시 무한 반복·취소 무시
  - 재연결 전 VPN 서버 route 갱신 불가
  - 비정상 종료 시 대기 패킷 미해제

## Windows
- 프로세스 강제 종료
  - 터널 어댑터의 숨은 상태 잔류, 다음 실행 시 삭제
  - 실행마다 다른 어댑터 이름
  - 강제 종료 직후 재로그인 가능
- Wintun 로그 콜백
  - 대상 세션: 프로세스 전체에 포인터 하나, 잠금 없음
  - 다시 연결로 두 세션이 겹치면 한 세션의 정리가 다른 세션의 포인터를 비움
  - Helper 비정상 종료 시 우선 확인 대상

# 앱↔Helper 통신

## macOS
- NSXPC 코드 서명 요구 조건: 메시지마다 보낸 프로세스의 서명 확인
  - 보낸 프로세스 종료 뒤 도착한 메시지 거부(오류 4102)
  - 영향: 앱의 응답 수신 전 Helper 종료 시 응답 유실

# 예외 규칙

## macOS
- networksetup 저장 additional routes 내 macOS 자체 추가 항목 부재
- macOS 자체 route(자기 IP /32, 169.254.0.0/16)는 실행 중 routing table에만 존재, networksetup 미표시

## Windows
- persistent route
  - 다시 시작한 뒤에도 유지
  - 어댑터 끄면 제외, 다시 켜면 자동 복귀
  - VPN 정상 종료·강제 종료 뒤에도 유지

# Windows UI
- NativeAOT 빌드: 코드에서 테마 리소스 CornerRadius 형 변환 시 InvalidCastException
  - 다른 값 형식 리소스(Thickness, double 등)의 같은 문제 여부 미확인
  - 대안: XAML Style·ThemeResource로 지정

# Windows 서비스·설치
- 서비스 시작 기한: 서비스 프로세스의 서비스 관리자 연결 기본 30초
  - 기한 초과 시 서비스 시작 실패
  - MSI의 대기 안 함 설정(Wait="no")도 "시작 중" 보고까지 대기
- 서비스 시작 기한 변경: PC 전체 설정(ServicesPipeTimeout) 한정
  - 재부팅 뒤 반영
- Helper가 실행한 설치 프로그램: 시스템 계정 실행
- 설치 패키지가 실행하는 프로그램: 시스템 계정 실행
  - 사용자 세션의 사용자 권한 앱 실행 불가
- 프로세스 번호: 끝난 뒤 재사용 가능
- sysinfo 프로세스 시작 시각: 초 단위

# GitHub
- releases/latest: 초안 릴리스 미포함
- releases/latest: 사전 공개 릴리스 미포함
- Actions macOS arm64 실행 환경 부족 시 job 미배정 취소 가능
  - 릴리즈 workflow: pkg job 취소 시 게시 단계 미실행
  - 대응: 실패 job 재실행
