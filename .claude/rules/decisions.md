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

# 설치·배포
- 버전 번호의 pre-release 꼬리표 금지
  - 사유: macOS·MSI 버전 칸이 숫자 3개만 허용

## macOS
- 번들 ID 변경 금지
  - 사유: Helper 등록이 Team ID와 번들 ID에 묶여 있어, 바꾸면 설치된 Mac마다 Helper 재등록·재승인 필요
