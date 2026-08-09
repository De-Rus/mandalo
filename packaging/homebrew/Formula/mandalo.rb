class Mandalo < Formula
  desc "Fast, offline, git-native API client — HTTP, GraphQL and gRPC from the terminal"
  homepage "https://mandalo.dev"
  version "0.2.0"
  license "MIT"

  # macOS gets the cask, not the formula: a formula without a bottle takes
  # Homebrew's build-from-source path, which demands current Command Line Tools
  # to install a binary that is already compiled. The cask also carries the
  # desktop app, and linking bin/mandalo from both would collide.
  depends_on :linux

  on_linux do
    on_arm do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_SHA256_aarch64-unknown-linux-gnu"
    end
    on_intel do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "REPLACE_WITH_SHA256_x86_64-unknown-linux-gnu"
    end
  end

  def install
    bin.install "mandalo"
  end

  test do
    assert_match "mandalo #{version}", shell_output("#{bin}/mandalo --version")
  end
end
