class Decide < Formula
  desc "Choice, score, and noul decisions via TypeSafe Jev or a local model"
  homepage "https://github.com/sparktype/decide"
  url "https://github.com/sparktype/decide/releases/download/v0.6.0/decide-v0.6.0-aarch64-apple-darwin.tar.gz"
  sha256 "fce59a3a6588dd79244a492202ddc9524d1572b1fa26c266ddf2d8c56a52e2de"
  version "0.6.0"

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
