class Mandalo < Formula
  desc "Fast, offline, git-native API client — HTTP, GraphQL and gRPC from the terminal"
  homepage "https://mandalo.dev"
  version "0.2.3"
  license "MIT"

  # macOS gets the cask, not the formula: a formula without a bottle takes
  # Homebrew's build-from-source path, which demands current Command Line Tools
  # to install a binary that is already compiled. The cask also carries the
  # desktop app, and linking bin/mandalo from both would collide.
  depends_on :linux

  on_linux do
    on_arm do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "08ada6b9c656cc5d9267b2630d22b771c4431b1dac1f58d5a43a1aa2db0bbe66"
    end
    on_intel do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "5a40c31514b792801662d260889da7afa8fcc079308e5d89acccb692fd06eaac"
    end
  end

  def install
    bin.install "mandalo"
  end

  test do
    assert_match "mandalo #{version}", shell_output("#{bin}/mandalo --version")
  end
end
