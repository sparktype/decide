# 게이트 사전 필터 확대 계획

## 왜 하나
사용 로그(230건)에서 모델이 정상적인 읽기 명령을 `ask`로 되묻거나(`docker ps` ask 67%) `deny`로 거부했다(`grep -rn "rm -rf /" docs/` deny 75%). 사전 필터는 모델을 부르지 않고 건너뛰는 명령 목록인데 12개뿐이라(`git status`, `ls`, `cat` 등) 이런 읽기 명령이 빠져 있다. 사전 필터에 걸리면 (1) 오탐·마찰이 사라지고 (2) 판정당 약 1초의 지연도 사라진다. 정적 규칙은 실제 로그에서 한 건도 걸리지 않아 지연을 줄이지 못했고(이전 작업의 정정), 지연을 줄이는 수단은 사전 필터다.

## 범위
- 기본 사전 필터에 **읽기 전용 명령**을 더한다. 기준: 인자로 파일을 쓰거나 지우거나 보낼 수 없고, 목적상 비밀을 드러내지 않는 명령.
- 사전 필터가 건너뛰는 읽기 명령이 비밀 파일을 읽는 틈을 막는다. `.env` 읽기 규칙의 읽기 명령 목록에 `grep`, `rg`, `jq`를 더한다(정적 규칙이 사전 필터보다 먼저라서 막힌다).
- 제외(후속): 기존 사전 필터의 `git branch`가 `git branch -D …` 같은 삭제도 건너뛰는 문제, 사전 필터 매칭 방식 변경.

## 더할 항목(기본 안, 구현 단계에서 테스트로 확정)
`grep`, `rg`, `jq`, `tree`, `file`, `stat`, `du`, `df`, `ps`, `whoami`, `date`, `uname`, `echo`, `docker ps`, `docker images`, `kubectl get pods`, `kubectl get nodes`, `kubectl get services`, `kubectl get deployments`, `kubectl get namespaces`, `git rev-parse`, `git ls-files`, `git blame`, `git shortlog`, `git describe`, `git stash list`, `git remote -v`.
- 뺀 것과 이유: `find`(`-delete`, `-exec`), `sort`·`uniq`(`-o`로 쓰기), `sed`·`awk`(쓰기·실행 가능), `env`(비밀 출력), `kubectl get`(`kubectl get secret -o yaml`이 비밀을 낸다), `git remote`·`git tag`·`git config`(설정 변경), `docker logs`(로그의 비밀).
- 사전 필터는 앞부분(단어 경계)이 맞고 셸 메타문자(`;&|`$<>()` 줄바꿈 역슬래시)가 없을 때만 건너뛴다는 기존 규칙을 그대로 쓴다. 정적 규칙이 사전 필터보다 먼저다.

## 검증 계획
1. **안전 회귀 방지(확정 검사, 기본 스위트).** 네 세트에서 ask·deny 라벨 명령이 규칙에도 안 걸리고 사전 필터로 조용히 건너뛰어지는 일이 없다. 위험한 근접 사례(`grep x > out`, `kubectl get secret -o yaml`, `find . -delete`, `docker rm`, `git remote add` 등)와 `grep SECRET .env`가 건너뛰어지지 않는다는 표 테스트.
2. **효과(실제 로그).** 재생 도구로 실제 `gate.log`에서 (a) 새로 사전 필터에 걸려 모델을 안 부르게 되는 호출의 수와 비율, (b) 그 호출들이 지금까지 받은 판정과 절약되는 지연, (c) 그중 사라지는 `ask`·`deny` 수를 잰다. 평가 세트로는 효과를 주장하지 않는다(내가 오류를 알고 고른 명령이라서).
3. 새로 건너뛰는 명령 목록을 사람이 읽고 위험한 것이 없는지 본다.

## 위험과 완화
| 위험 | 완화 |
|---|---|
| 읽기 명령이 비밀을 읽고 건너뛰어짐(`grep SECRET .env`) | `.env` 읽기 규칙에 `grep`·`rg`·`jq` 추가, 개인 키·자격증명 규칙은 이미 명령과 무관하게 건다 |
| 목록에 쓰기 가능한 명령이 섞임 | 기준(쓰기·삭제·전송 불가)을 문서화하고 위험 근접 사례 표 테스트 |
| 사용자 설정이 목록을 통째로 바꿈 | 기존 동작 그대로(사용자 층은 교체, 저장소 층은 줄이기만) |
