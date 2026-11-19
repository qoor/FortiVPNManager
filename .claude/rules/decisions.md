---
paths:
  - "components/**"
  - "packaging/**"
  - ".github/**"
  - "VERSION"
  - "docs/SPEC.md"
---

# OTP
- 수동 OTP 입력 창 미제공
  - 사유: OTP 입력 자동화가 목적

# Helper

## Windows
- 보안 프로그램 등 환경별 특성을 근거로 한 설계·설명 금지
  - 사유: 특정 PC 환경이 아닌 일반 사용 기준으로 설계
  - 대기·재시도는 일반 규칙으로 설명
- wintun.dll: Helper 시작 때 한 번 로드, 해제 없음
  - 사유: openconnect가 세션이 끝날 때마다 DLL을 해제하는데, 이때 Wintun의 남은 어댑터 정리가 같은 DLL 코드를 실행 중일 수 있음
  - 해제되면 Helper 비정상 종료 위험
  - openconnect 수정 없이 참조 횟수만으로 해제 방지
- 의도된 Helper 재시작: 종료 코드 4로 프로세스 종료, 서비스 중지 미보고
  - 대상: 세션 정리 멈춤, 내부 멈춤, 패닉
  - 사유: abort()의 종료 상태(0xC0000409)는 스택 버퍼 오버런으로 기록되어, 의도된 재시작이 메모리 손상 크래시로 보임
  - 재실행: 서비스 관리자의 실패 시 재시작 설정
  - 4를 고른 이유: 1·2는 Helper가 이미 사용, 3은 C 런타임 abort()의 종료 코드
  - macOS: abort() 유지, launchd가 재시작

# 설치·배포
- 버전 번호의 pre-release 꼬리표 금지
  - 사유: macOS·MSI 버전 칸이 숫자 3개만 허용

## macOS
- 번들 ID 변경 금지
  - 사유: Helper 등록이 Team ID와 번들 ID에 묶여 있어, 바꾸면 설치된 Mac마다 Helper 재등록·재승인 필요
