# decide-view 모드 체크리스트

- [x] 계획·결정 기록 (`context-notes.md`)
- [x] 모드 뼈대 (`plugin.json`, `hooks.json`, 타입 계약)
- [x] 1. 판정 상태 줄 (백엔드·모델·지연·데몬 생존)
- [x] 2. 게이트 판정 토스트 (ask·deny)
- [x] 3. `/decide-stats` 패널 (`decide gate stats` 출력)
- [x] 4. 판정 결과 밴드 (`decide`·`decide_many` 결과)
- [x] 단위 테스트 (`claude plugin test`)
- [x] `claude plugin validate` 통과
- [ ] 핫리로딩 활성화 후 실제 세션에서 확인 (패널 막대 그래프 변경 후 재확인 필요)

경로 `~/.claude/dev-mods/00929dfc-6328-4a43-9cb6-94594948beed/decide-view/`. 타입 검사(`tsc`)는 설치돼 있지 않아 하지 않았다.
