// 명령줄 도움말과 버전 문구를 만든다(명령 목록을 한 곳에서 관리해 도움말이 분기와 어긋나지 않게 한다)
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `(명령, 한 줄 설명)`. `main.rs`의 분기와 같은 이름이어야 하며 테스트가 모든 명령의 도움말을 확인한다.
pub const COMMANDS: [(&str, &str); 5] = [
    ("mcp", "stdio MCP 서버를 실행한다"),
    ("daemon", "판정 요청을 받는 상주 데몬을 실행한다"),
    ("install", "Claude Code에 decide를 등록한다"),
    ("hook", "Claude Code PostToolUse 훅: decide 결과를 한 줄로 보여준다"),
    ("gate", "Claude Code PreToolUse 훅: Bash 명령을 decide로 판정한다"),
];

/// `decide --version`이 내는 한 줄.
pub fn version_line() -> String {
    format!("decide {VERSION}")
}

/// `decide`, `decide -h`, `decide --help`, `decide help`가 내는 전체 도움말.
pub fn overview() -> String {
    let commands = COMMANDS
        .iter()
        .map(|(name, summary)| format!("  {name:<10}{summary}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\
decide {VERSION} - choice, score, noul 판단을 내리는 MCP 도구

사용법:
  decide <명령> [옵션]
  decide -h | --help | -V | --version

명령:
{commands}
  help      명령의 도움말을 보여준다(decide help <명령>)

옵션:
  -h, --help       도움말을 보여준다(명령 뒤에 쓰면 그 명령의 도움말)
  -V, --version    버전을 보여준다

환경 변수:
  DECIDE_BACKEND     typesafe 또는 local. 백엔드를 고른다
  TYPESAFE_API_KEY   있고 백엔드를 안 고르면 TypeSafe Jev를 쓴다
  DECIDE_LOCAL_URL   local 서버 주소(기본 http://127.0.0.1:8009/v1/systemone)

예:
  decide install --claude    MCP 등록과 훅 설치를 한 번에 한다
  decide gate --show         게이트 목록과 설정 위치를 본다
  decide daemon &            상주 데몬을 띄운다

인자 없이 실행하면 이 도움말을 보여주고 아무것도 시작하지 않는다.
자세한 사용법: decide <명령> --help"
    )
}

/// `decide <명령> --help`, `decide help <명령>`이 내는 명령별 도움말. 알 수 없는 명령이면 `None`이다.
pub fn for_command(name: &str) -> Option<String> {
    let text = match name {
        "mcp" => {
            "\
사용법: decide mcp

stdio로 MCP 서버를 실행한다.
`decide install`이 Claude Code에 등록하는 명령이 이것이다.
제공 도구: decide, decide_many

환경 변수는 decide --help를 보라."
        }
        "daemon" => {
            "\
사용법: decide daemon

판정 요청을 JSON 한 줄씩 받는 상주 데몬을 실행한다.
  소켓   ~/.cache/decide/decide.sock
  종료   30분 동안 요청이 없으면 끝난다. 요청의 client_version이
         이 바이너리의 버전과 다르면 stale로 답하고 끝난다.
         업그레이드한 클라이언트가 옛 데몬과 말하지 않게 하려는 것이다.
  캐시   같은 요청은 데몬이 사는 동안 최대 64개까지 캐시한다.
이미 데몬이 떠 있으면 새로 띄우지 않고 끝난다.
게이트 훅이 쓰는 것이라 MCP는 이 데몬이 아니라 decide mcp(stdio)로 간다."
        }
        "install" => {
            "\
사용법: decide install [--claude]

Claude Code 사용자 스코프에 decide MCP 서버를 stdio(decide mcp)로
등록한다. 이미 등록돼 있으면 그대로 두고 성공으로 끝난다. 예전 HTTP
등록을 옮기려면 claude mcp remove -s user decide 뒤에 다시 실행한다.

옵션:
  --claude   MCP 등록에 더해 훅도 사용자 설정(settings.json)에 넣는다.
             결과 표시 훅(PostToolUse)과 bash-risk 게이트 훅
             (PreToolUse, Bash)이다. 게이트는 기본이 감사 모드라
             아무것도 막지 않는다. 여러 번 실행해도 안전하고,
             바꾸기 전에 settings.json.bak-decide로 백업한다."
        }
        "hook" => {
            "\
사용법: decide hook

Claude Code PostToolUse 훅이다. stdin의 훅 입력에서 decide의
질문과 결과를 읽어 한 줄 요약(systemMessage)으로 보여준다.
읽을 수 없는 입력에는 아무것도 내지 않고 종료 코드 0이라
에이전트 작업을 막지 않는다. 보통 decide install --claude가 등록한다."
        }
        "gate" => {
            "\
사용법:
  decide gate <이름>
  decide gate --show [이름] [--json]
  decide gate stats [--since 24h|7d|all] [--json]

Claude Code PreToolUse 훅에서 decide로 판정한다.
게이트: bash-risk (Bash 명령의 위험 판정)

  gate <이름>    stdin의 훅 입력을 판정해 훅 출력 JSON을 낸다.
                 어떤 실패도 판정 없이 통과하고 종료 코드 0이다.
                 알 수 없는 게이트 이름만 종료 코드 1이다.
  --show         게이트 목록과 설정 파일 위치를 보여준다.
  --show <이름>  그 게이트의 질문, 선택지, 임계값, 사전 필터,
                 정적 규칙과 값마다의 출처를 보여준다.
  --json         --show <이름>과 함께 쓴다. 설정 파일에 그대로
                 복사할 수 있는 JSON으로 낸다.
  stats          감사 로그(gate.log)를 집계해 판정 방식, 판정 분포,
                 지연, 시간 초과, 규칙 적중을 보여준다. --since로
                 기간(24h, 7d, all)을 거르고 --json은 JSON으로 낸다.
                 로그를 바꾸지 않는다.

정적 규칙: gates.bash-risk.deny_patterns, ask_patterns(글롭, *만
지원)에 걸리는 명령은 모델을 부르지 않고 deny 또는 ask로 정한다.
사전 필터보다 먼저 본다.

기본은 감사 모드라 아무것도 막지 않는다. 판정과 근거를 보여주고
~/.cache/decide/gate.log에 남긴다.
설정: ~/.config/decide/gates.json, ./.decide/gates.json
(저장소 설정은 더 엄격한 쪽으로만 바꿀 수 있다)"
        }
        _ => return None,
    };
    Some(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_line_is_the_name_and_the_crate_version() {
        assert_eq!(version_line(), format!("decide {}", env!("CARGO_PKG_VERSION")));
        assert_eq!(VERSION, crate::daemon::VERSION, "데몬의 버전과 같은 값이어야 한다");
    }

    #[test]
    fn the_overview_has_the_conventional_sections_in_order() {
        let text = overview();
        let positions: Vec<usize> = ["사용법:", "명령:", "옵션:", "환경 변수:", "예:"]
            .iter()
            .map(|section| text.find(section).unwrap_or_else(|| panic!("{section} 섹션이 없다:\n{text}")))
            .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "섹션 순서가 어긋났다:\n{text}");
        assert!(text.starts_with(&format!("decide {VERSION}")), "첫 줄에 이름과 버전이 있어야 한다:\n{text}");
    }

    #[test]
    fn the_overview_documents_the_options_and_the_environment() {
        let text = overview();
        for needle in [
            "-h, --help",
            "-V, --version",
            "DECIDE_BACKEND",
            "TYPESAFE_API_KEY",
            "DECIDE_LOCAL_URL",
            "decide <명령> --help",
            "decide install --claude",
        ] {
            assert!(text.contains(needle), "{needle}가 없다:\n{text}");
        }
    }

    #[test]
    fn every_command_is_listed_and_has_its_own_help() {
        let text = overview();
        for (name, summary) in COMMANDS {
            assert!(text.contains(&format!("  {name}")), "{name}이 명령 목록에 없다:\n{text}");
            assert!(text.contains(summary), "{name}의 설명이 없다");
            let help = for_command(name).unwrap_or_else(|| panic!("{name}의 도움말이 없다"));
            assert!(help.starts_with("사용법:"), "{name}: {help}");
            assert!(help.contains(&format!("decide {name}")), "{name}: {help}");
        }
    }

    #[test]
    fn an_unknown_command_has_no_help() {
        assert!(for_command("nope").is_none());
        assert!(for_command("").is_none());
        assert!(for_command("--help").is_none());
    }

    #[test]
    fn the_command_help_texts_explain_their_options() {
        let install = for_command("install").unwrap();
        assert!(install.contains("--claude") && install.contains("settings.json"), "{install}");
        let gate = for_command("gate").unwrap();
        for needle in [
            "bash-risk", "--show", "--json", "감사 모드", "gates.json", "decide gate stats", "--since", "gate.log",
            "24h",
        ] {
            assert!(gate.contains(needle), "{needle}가 없다:\n{gate}");
        }
        let daemon = for_command("daemon").unwrap();
        assert!(daemon.contains("decide.sock") && daemon.contains("30분"), "{daemon}");
        assert!(for_command("mcp").unwrap().contains("stdio"));
        assert!(for_command("hook").unwrap().contains("PostToolUse"));
    }

    #[test]
    fn help_lines_stay_readable_in_a_terminal() {
        // 한글은 터미널에서 두 칸을 차지하므로 글자 수가 아니라 대략 80칸 안쪽으로 본다.
        let width = |line: &str| line.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum::<usize>();
        let mut texts = vec![overview()];
        texts.extend(COMMANDS.iter().map(|(name, _)| for_command(name).unwrap()));
        let too_long: Vec<String> = texts
            .iter()
            .flat_map(|text| text.lines())
            .filter(|line| width(line) > 80)
            .map(|line| format!("{}칸: {line}", width(line)))
            .collect();
        assert!(too_long.is_empty(), "80칸을 넘는 줄:\n{}", too_long.join("\n"));
    }
}
