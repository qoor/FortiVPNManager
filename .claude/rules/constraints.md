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

# openconnect
- 끊김 감지
  - 어댑터가 꺼지면 즉시 감지
  - 게이트웨이와의 통신만 끊기면 약 20초 뒤 감지
  - 재시도 중에도 터널 어댑터는 Up 상태 유지
  - 끊김 확인: 콜백·종료 코드 기준
- logout
  - logout 요청의 응답은 가짜 cookie에도 같아 성공 여부 판단 불가
  - 세션 정보 요청 응답으로 판단 가능: 터널이 열린 세션 403, 끝난 세션 302
  - 시간 제한 부재
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

# 예외 규칙

## macOS
- networksetup 저장 additional routes 내 macOS 자체 추가 항목 부재
- macOS 자체 route(자기 IP /32, 169.254.0.0/16)는 실행 중 routing table에만 존재, networksetup 미표시

## Windows
- persistent route
  - 다시 시작한 뒤에도 유지
  - 어댑터 끄면 제외, 다시 켜면 자동 복귀
  - VPN 정상 종료·강제 종료 뒤에도 유지
