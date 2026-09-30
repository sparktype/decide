class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/releases/download/v0.0.5/decide-v0.0.5-aarch64-apple-darwin.tar.gz"
  sha256 "5d2c757ea13f4495e3c83fcea2ce6a3cf3d6a66ef8e6550947ad82b3416ea6f1"
  version "0.0.5"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  def install
    bin.install "decide"
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
  end
end
