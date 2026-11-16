# Rust 공통 구현 도입 리팩토링 설계
- 상태: 완료(2026-11-09), 이후 갱신 없음

# 1. 목적
- 다른 OS·VPN client 추가 시 UI·OS 연동만 새로 작성하는 구조
- 공통 부분(판단, 공통 I/O, VPN client 처리)의 Rust 이전

# 2. 원칙
- 사용자가 보는 동작 유지
  - 승인된 예외
    - config.yaml의 raw NEL·LS·PS 오류 처리
    - YAML parser 교체에 따른 일부 오류 위치·드문 문법 처리 차이(값 해석은 동일)
    - VPN client 실행 파일 문제의 종류·경로 전달, 문장은 UI가 생성
    - Rust 쪽 통신·파일 오류의 라이브러리 원문(영어) 표시
- 저장 데이터 호환: config.yaml, state.json, Keychain 항목, 로그 파일
- 기존 코드 참고 범위: 바꿀 수 없는 외부 사실 확인 한정
  - 기존 구조·흐름 미답습, 설계에서 출발

# 3. 구조 결정
- 공통 로직: Rust
- OS 영역(UI, OS 연동, Helper OS 영역): 각 OS 기본 언어
  - 사유: OS 기본 UI의 단순성과 반응 속도, OS 의존 영역의 다른 언어 이전은 복잡도만 증가
- 디렉터리: 구성 요소 기준 분리(언어별·플랫폼별 배치 미채택)
- VPN client별 처리(backend adapter)와 프로세스 처리의 분리
- Swift ↔ Rust: 프로세스 안 socketpair 위 길이 + JSON 메시지
  - UniFFI 미채택 사유: C binding용 unsafe 증가, 설정 부담
- 메시지 타입 기준: Rust, Swift 쪽은 Rust 생성 fixture로 검증
- UI 상태: 도메인 구역 단위 전송(화면 모양의 전체 상태 미사용)
- Rust unsafe 블록 금지(외부 시작 함수의 이름 표시만 허용)
- 내부 오류: 즉시 중단(panic abort), crash report 확보
