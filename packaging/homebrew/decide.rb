class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local System One server"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/releases/download/v0.8.0/decide-v0.8.0-aarch64-apple-darwin.tar.gz"
  sha256 "9803b11d0ce23d6f731ef29cd09ac4dfd85a33f53d89e8fbc7511a3961549c99"
  version "0.8.0"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  def install
    bin.install "decide"
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
  end
end
