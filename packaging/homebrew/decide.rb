class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/releases/download/v0.0.4/decide-v0.0.4-aarch64-apple-darwin.tar.gz"
  sha256 "c875a5df8e27d725edd2e16d26c46dc6c7aed59c6987747dfe7f04944b89e41c"
  version "0.0.4"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  def install
    bin.install "decide"
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
  end
end
