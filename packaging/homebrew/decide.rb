class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/archive/refs/tags/v0.0.1.tar.gz"
  sha256 "78d1a9e39009e8ad766263fb195dd5e1692ded6b7cd7879b232360d02e09ae4a"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/decide")
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
  end
end
