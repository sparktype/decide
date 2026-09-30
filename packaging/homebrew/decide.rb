class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/releases/download/v0.0.6/decide-v0.0.6-aarch64-apple-darwin.tar.gz"
  sha256 "ad54f562b8cf6de06a91a190ef68cf08371a15de961a4549a6cf97b59d6edd42"
  version "0.0.6"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  def install
    bin.install "decide"
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
  end
end
