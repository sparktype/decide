class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/archive/refs/tags/v0.0.1.tar.gz"
  version "0.0.1"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/decide")
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
  end
end
