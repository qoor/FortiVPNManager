---
name: manual-check
description: 실설치 점검 절차
---

# 1. 준비
- 첫 실행 전 기존 openfortivpn 설정의 비밀번호 최신 여부 확인
  - 사유: 가져오기 시 Keychain 비밀번호 덮어쓰기

# 2. 설치
- 설치 화면 언어: OS 언어 기준(한국어·영어)
- 연결 중 업그레이드: 앱 종료 후 설치
- 업그레이드 후 새 Helper로 연결
- 같은 버전 재설치 가능, 낮은 버전 설치 거부

## macOS
- pkg 실행: 차단 후 "그래도 열기"로 실행
- 설치 위치: /Applications 한정, 빌드 폴더 앱 미변경
- 설치된 앱의 quarantine 표시 부재
- 명령줄 설치(sudo installer): 실행 중인 앱 종료 후 설치

## Windows
- 화면 설치와 quiet 설치(msiexec /qn)의 동일 결과
- 설치 직후 Helper 서비스 정지 상태
- 업그레이드 시 "사용 중인 파일" 창·재시작 요청 부재
- 설치 실패 시 이전 버전 복귀
- 같은 버전 다른 빌드 설치 후 설정 > 앱의 항목 하나 유지
- 일반 사용자 계정의 앱 실행: Helper 서비스 시작 후 연결

# 3. 첫 실행
- 인증서 값을 비운 연결: 거부 1회 후 즉시 재연결, 값 저장

## macOS
- config.yaml 없는 첫 실행: openfortivpn 설정 가져오기 후 연결

## Windows
- Mac config.yaml 복사 후 첫 실행: 비밀번호·Gmail 인증 입력만으로 연결
- 앱 실행: Helper 서비스 실행 후 알림 영역 아이콘 표시

# 4. 정상 동작
- 세션 최대 시간 단축 후 사전 재연결 동작
- config.yaml 직접 편집
  - 변경 반영
  - 잘못된 값 거부
  - 깨진 YAML의 마지막 정상 설정 유지
  - symlink 유지
- 잠자기(짧은·긴·연결 중)·Wi-Fi 변경 후 연결 상태

## Windows
- [연결 해제]·사전 재연결 후
  - logout 요청 로그
  - 게이트웨이 host route 제거
  - openconnect·route 스크립트 프로세스 잔류 부재
- 예외 규칙: 유선·Wi-Fi별 직접 경로, VPN 재연결 후 유지

# 5. 장애·복구
- 잘못된 VPN 비밀번호·Gmail 인증, OTP 메일 차단: 멈춤 상태 진입

## macOS
- openfortivpn SIGKILL 종료: route 복구 후 즉시 재연결
- pppd 단독 종료: 서버 종료와 같은 대기 후 재연결
- 연결 후 2분 안 종료: 더 긴 대기 후 재연결
- 앱 강제·정상 종료: 프로세스 정리, route 복원
- 연결 중 Helper 강제 종료(kill -9): 이전 실행 정리 후 즉시 재연결

## Windows
- 작업 관리자로 openconnect 종료: 게이트웨이 host route 정리, 즉시 재연결
- Helper 서비스 강제 종료: 서비스 자동 재시작 후 재연결
- 네트워크 일시 끊김: 복귀 후 재연결
- Helper 서비스 사용 안 함 상태의 앱 실행: 오류 창 표시 후 앱 종료
- Helper 서비스 시작 2회 실패: 두 번째 실패만 오류 창 표시

# 6. 보안
- 프로세스 명령줄·로그의 비밀값 부재

## macOS
- openfortivpn 임시 설정 파일: 권한 0600, 종료 후 삭제
- config.yaml·state.json 권한 0600
- make verify 통과

# 7. 정리

## Windows
- 제거(두 사용자가 예외 규칙을 쓴 PC)
  - 앱이 쓴 예외 route·로그인 시 열기 제거
  - 밖에서 넣은 route, 설정·상태 파일, 로그, 저장된 비밀값 유지
  - Helper 서비스, 실행 기록 폴더, 시작 메뉴 바로 가기, 이벤트 로그 출처 제거
- Repair: 지운 앱 파일 복구
