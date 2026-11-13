class Rxform < Formula
  desc "Convert XLSForm spreadsheets to ODK XForm XML (Rust port of pyxform)"
  homepage "https://milkway.github.io/rxform/"
  version "0.1.5"
  license "BSD-2-Clause"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/milkway/rxform/releases/download/v0.1.5/rxform-v0.1.5-aarch64-apple-darwin.tar.gz"
      sha256 "50884021ba352e1ab1102cbda69d775255a6faf9ec7d10645b475b5d01c8a53c"
    else
      url "https://github.com/milkway/rxform/releases/download/v0.1.5/rxform-v0.1.5-x86_64-apple-darwin.tar.gz"
      sha256 "c8945d0a48f8cbbec995b62df4202ef4fbae1971cbf6a70d4f80047467b19fdb"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/milkway/rxform/releases/download/v0.1.5/rxform-v0.1.5-aarch64-unknown-linux-musl.tar.gz"
      sha256 "3d13302f3ab25883ed3a67ae63355740f472eb151689bf6d870de94cfbfe8c78"
    else
      url "https://github.com/milkway/rxform/releases/download/v0.1.5/rxform-v0.1.5-x86_64-unknown-linux-musl.tar.gz"
      sha256 "3a488cce159673b9a5b7a99e81646838f15ebc9dd097ea868d063c3ca3a2b6d4"
    end
  end

  def install
    bin.install "rxform"
  end

  test do
    assert_match "rxform", shell_output("#{bin}/rxform --version")
  end
end
