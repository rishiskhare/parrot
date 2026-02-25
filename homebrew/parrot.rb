cask "parrot" do
  version "26.2.4"

  sha256 "REPLACE_WITH_ARM_DMG_SHA256"
  url "https://github.com/rishiskhare/parrot/releases/download/v#{version}/Parrot_#{version}_aarch64.dmg"

  name "Parrot"
  desc "Text-to-speech desktop app powered by Kokoro"
  homepage "https://github.com/rishiskhare/parrot"

  app "Parrot.app"

  zap trash: [
    "~/.config/parrot",
    "~/Library/Application Support/com.rishiskhare.parrot",
    "~/Library/Preferences/com.rishiskhare.parrot.plist",
    "~/Library/Logs/com.rishiskhare.parrot",
    "~/Library/WebKit/com.rishiskhare.parrot",
  ]
end
