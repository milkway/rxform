class Rxform < Formula
  desc "Convert XLSForm spreadsheets to ODK XForm XML (pyxform in Rust)"
  homepage "https://milkway.github.io/rxform/"
  version "0.1.2"
  license "BSD-2-Clause"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/milkway/rxform/releases/download/v0.1.2/rxform-v0.1.2-aarch64-apple-darwin.tar.gz"
      sha256 "758f85e2940a58fc2a4f6047ca182c53d0803714a0798a8a572ae869071a17d0"
    else
      url "https://github.com/milkway/rxform/releases/download/v0.1.2/rxform-v0.1.2-x86_64-apple-darwin.tar.gz"
      sha256 "44890c2f07bed73ea056b079673049fe16c1f967b033153d8ca3ecd86a9e43ef"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/milkway/rxform/releases/download/v0.1.2/rxform-v0.1.2-aarch64-unknown-linux-musl.tar.gz"
      sha256 "231c9e6098a27cb174075f1baabc86f0fcdad72f6a52e594cc2076e605a3777e"
    else
      url "https://github.com/milkway/rxform/releases/download/v0.1.2/rxform-v0.1.2-x86_64-unknown-linux-musl.tar.gz"
      sha256 "b92f3c030b603c33bc606affe8d02f1e7169dcbdefcfa83df1b3e1760d7e158f"
    end
  end

  def install
    bin.install "rxform"
  end

  test do
    assert_match "rxform", shell_output("#{bin}/rxform --version")
  end
end
