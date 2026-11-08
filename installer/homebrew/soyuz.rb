# Homebrew cask template, rendered by .github/workflows/package-manifests.yml
# (placeholders: @VERSION@, @SHA256_MACOS_ARM64@). For a tap such as ulaas/homebrew-tap:
#   brew install --cask ulaas/tap/soyuz
# The tarball has one top-level folder holding soyuz, soyuz-tui and the FFmpeg dylibs
# (loaded from next to the executable), see scripts/package.ps1.
cask "soyuz" do
  version "@VERSION@"
  sha256 "@SHA256_MACOS_ARM64@"

  url "https://github.com/ulaas/soyuz/releases/download/v#{version}/soyuz-#{version}-macos-arm64.tar.gz"
  name "Soyuz"
  desc "Keyboard-first file explorer"
  homepage "https://ulaas.github.io/soyuz/"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64

  binary "soyuz-#{version}-macos-arm64/soyuz"
  binary "soyuz-#{version}-macos-arm64/soyuz-tui"

  zap trash: "~/Library/Application Support/soyuz"

  caveats <<~EOS
    Soyuz release builds are not code-signed or notarized yet. If macOS refuses to start
    `soyuz` or `soyuz-tui`, clear the quarantine flag once:
      xattr -dr com.apple.quarantine "#{staged_path}"
  EOS
end
