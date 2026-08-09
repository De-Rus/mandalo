class Mandalo < Formula
  desc "Fast, offline, git-native API client — HTTP, GraphQL and gRPC from the terminal"
  homepage "https://mandalo.dev"
  version "0.2.1"
  license "MIT"

  # macOS gets the cask, not the formula: a formula without a bottle takes
  # Homebrew's build-from-source path, which demands current Command Line Tools
  # to install a binary that is already compiled. The cask also carries the
  # desktop app, and linking bin/mandalo from both would collide.
  depends_on :linux

  on_linux do
    on_arm do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "b2e44f3e3fc4eedd2ff5be2502c2250cc56db23c54b163feae02eb660fab6441"
    end
    on_intel do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "d72488aef1f8a5f0736a19bd66e93063f959240a78d804b48c015008b223e68a"
    end
  end

  def install
    bin.install "mandalo"
  end

  test do
    assert_match "mandalo #{version}", shell_output("#{bin}/mandalo --version")
  end
end
