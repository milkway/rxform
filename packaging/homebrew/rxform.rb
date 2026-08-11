class Rxform < Formula
  desc "Convert XLSForm spreadsheets to ODK XForm XML (pyxform in Rust)"
  homepage "https://milkway.github.io/rxform/"
  version "0.1.1"
  license "BSD-2-Clause"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/milkway/rxform/releases/download/v0.1.1/rxform-v0.1.1-aarch64-apple-darwin.tar.gz"
      sha256 "SHA_MAC_ARM"
    else
      url "https://github.com/milkway/rxform/releases/download/v0.1.1/rxform-v0.1.1-x86_64-apple-darwin.tar.gz"
      sha256 "SHA_MAC_X64"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/milkway/rxform/releases/download/v0.1.1/rxform-v0.1.1-aarch64-unknown-linux-musl.tar.gz"
      sha256 "SHA_LINUX_ARM"
    else
      url "https://github.com/milkway/rxform/releases/download/v0.1.1/rxform-v0.1.1-x86_64-unknown-linux-musl.tar.gz"
      sha256 "SHA_LINUX_X64"
    end
  end

  def install
    bin.install "rxform"
  end

  test do
    assert_match "rxform", shell_output("#{bin}/rxform --version")
  end
end
