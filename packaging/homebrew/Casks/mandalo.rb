cask "mandalo" do
  version "0.2.1"
  sha256 "2322dc5448573b332849cca17239728bf7f735f7c502941d80b1af967f1c735e"

  url "https://github.com/De-Rus/mandalo/releases/download/v#{version}/mandalo_#{version}_universal.dmg"
  name "Mándalo"
  desc "Offline, git-native API client — HTTP, GraphQL and gRPC"
  homepage "https://mandalo.dev/"

  # The app updates itself through the Tauri updater, so brew must not treat a
  # self-updated install as outdated and reinstall over it.
  auto_updates true
  depends_on macos: :catalina

  app "mandalo.app"
  # The CLI rides inside the bundle rather than being downloaded a second time.
  # It is `mandalo-cli` in there because Tauri refuses a sidecar named after the
  # src-tauri crate; target: puts it on PATH as `mandalo`. No conflicts_with is
  # needed: the formula is `depends_on :linux`, so it can never land here.
  binary "#{appdir}/mandalo.app/Contents/MacOS/mandalo-cli", target: "mandalo"

  # No postflight xattr strip: the app and the sidecar are Developer ID signed
  # and notarized, and the .dmg is stapled, so Gatekeeper clears them as they
  # are. See ../../docs/macos-signing.md.

  zap trash: [
    "~/Library/Application Support/com.drus.mandalo",
    "~/Library/Caches/com.drus.mandalo",
    "~/Library/Saved Application State/com.drus.mandalo.savedState",
    "~/Library/WebKit/com.drus.mandalo",
  ]

  caveats <<~EOS
    This installs both the app and the `mandalo` CLI, which share the workspace
    in ~/Mandalo — plain TOML collections and environments, yours to put in git.

    The app updates itself, so `brew upgrade --cask mandalo` is not how new
    versions arrive; Homebrew only reinstalls if you ask it to.

    `brew uninstall --cask mandalo` leaves ~/Mandalo alone. --zap clears the
    app's caches and state, and still does not touch your collections.
  EOS
end
