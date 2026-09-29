class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "file:///Users/spark/Develop/Workspaces/decide-mcp"
  # 원격이 없다. 이 체크아웃의 main을 클론해 실행 파일만 설치한다.
  # Homebrew 7: HOMEBREW_DEVELOPER=1 brew install --formula ./packaging/homebrew/decide.rb
  url "file:///Users/spark/Develop/Workspaces/decide-mcp", using: :git, branch: "main"
  version "0.1.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/decide")
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
  end
end
