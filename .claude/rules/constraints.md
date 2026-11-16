---
paths:
  - "components/**"
  - "packaging/**"
  - "docs/SPEC.md"
---

# 연결 관리
- 같은 계정의 동시 세션 허용

# OTP
- OTP 유효시간 약 60초
- 메일 도착 후 감지까지 걸린 시간
  - IMAP: 약 7초
  - Gmail API: 3~12초, 중앙값 약 4.6초

# VPN client

## openfortivpn
- 터널이 올라온 뒤 SIGTERM을 받으면 정상 종료(exit 0)
  - 서버가 끊은 경우와 구별 불가
  - signal 종료로 남기려면 SIGKILL 필요
- pppd만 끝내도 openfortivpn은 signal 없이 스스로 종료

## openconnect
- route 스크립트
  - 끝날 때까지 VPN 통신 불가
  - 스크립트가 끝났다는 출력 없음
- 끊김 감지
  - 어댑터가 꺼지면 즉시 감지
  - 게이트웨이와의 통신만 끊기면 약 20초 뒤 감지
  - 재시도 중에도 터널 어댑터는 Up 상태 유지
  - 끊김은 openconnect가 끝날 때에야 확인 가능
- logout
  - logout 요청의 응답은 가짜 cookie에도 같아 성공 여부 판단 불가
  - 세션 정보 요청 응답으로 판단 가능: 터널이 열린 세션 403, 끝난 세션 302

### macOS
- route 스크립트
  - 시간 제한 없이 대기
  - 스크립트 출력이 openconnect 출력으로 옴

### Windows
- route 스크립트
  - 10초가 지나면 스크립트를 둔 채 진행
  - 스크립트 출력이 openconnect 출력으로 오지 않음
- 종료 이벤트
  - Ctrl+C, console 닫힘, logoff, shutdown: logout과 route 복원 후 종료
  - Ctrl+Break: logout 없이 세션을 남긴 채 종료
- 강제 종료
  - 터널 어댑터는 숨은 상태로 남았다가 다음 실행이 지움
  - 어댑터 이름이 실행마다 다름
  - 강제 종료 직후 재로그인 가능

# Helper

## Windows
- 시스템 계정 서비스 안에서 띄운 프로세스에는 같은 서비스에서 보낸 Ctrl+C가 전달됨

# 예외 규칙

## macOS
- networksetup의 저장된 additional routes에 macOS가 스스로 넣는 항목 없음
- macOS 자체 route(자기 IP /32, 169.254.0.0/16)는 실행 중인 routing table에만 있고 networksetup에는 안 보임

## Windows
- persistent route
  - 다시 시작한 뒤에도 유지
  - 어댑터를 끄면 빠졌다가 켜면 저절로 돌아옴
  - VPN 정상 종료·강제 종료 뒤에도 유지
