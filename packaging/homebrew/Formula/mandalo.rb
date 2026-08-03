class Mandalo < Formula
  desc "Fast, offline, git-native API client — HTTP, GraphQL and gRPC from the terminal"
  homepage "https://mandalo.dev"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_aarch64-apple-darwin"
    end
    on_intel do
      url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_SHA256_x86_64-apple-darwin"
    end
  end

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
