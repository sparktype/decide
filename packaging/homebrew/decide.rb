class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/releases/download/v0.2.1/decide-v0.2.1-aarch64-apple-darwin.tar.gz"
  sha256 "8ba6d72c26157482d8eef0976b65bb0b1ded22c025c52eb7607c876f600b5340"
  version "0.2.1"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  def install
    # mlx.metallib(MLX GPU 커널)은 바이너리와 같은 디렉터리에 있어야 MLX가 찾는다.
    bin.install "decide", "mlx.metallib"
  end

  test do
    assert_match "mcp", shell_output("#{bin}/decide --help")
    assert_predicate bin/"mlx.metallib", :exist?
  end
end
