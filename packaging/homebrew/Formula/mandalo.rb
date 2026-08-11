class Mandalo < Formula
  desc "Fast, offline, git-native API client — HTTP, GraphQL and gRPC from the terminal"
  homepage "https://mandalo.dev"
  version "0.2.2"
  license "MIT"

  # macOS gets the cask, not the formula: a formula without a bottle takes
  # Homebrew's build-from-source path, which demands current Command Line Tools
  # to install a binary that is already compiled. The cask also carries the
  # desktop app, and linking bin/mandalo from both would collide.
  depends_on :linux

  on_linux do
    on_arm do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "45cac11093658f12b8b80698484bde54f7374c2208b665e3f45348225a32e5ed"
    end
    on_intel do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "cb884e451f5d4109e9b8d6f8c303fcb7698bbbb4b5d0ded3ec7148e8a9bd7ac8"
    end
  end

  def install
    bin.install "mandalo"
  end

  test do
    assert_match "mandalo #{version}", shell_output("#{bin}/mandalo --version")
  end
end
