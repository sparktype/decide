class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/releases/download/v0.0.3/decide-v0.0.3-aarch64-apple-darwin.tar.gz"
  sha256 "785c0fa8f40a943dee3abd342a31ad1a05bdceeaa3aa2646089a994e372c29ae"
  version "0.0.3"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  def install
    bin.install "decide"
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
  end
end
