import React from "react";
import { useTranslation } from "react-i18next";
import { type } from "@tauri-apps/plugin-os";
import { LogLevelSelector } from "./LogLevelSelector";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SoundPicker } from "../SoundPicker";
import { ShortcutInput } from "../ShortcutInput";
import { UpdateChecksToggle } from "../UpdateChecksToggle";

export const DebugSettings: React.FC = () => {
  const { t } = useTranslation();
  const isLinux = type() === "linux";

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.debug.title")}>
        <LogLevelSelector grouped={true} />
        <UpdateChecksToggle descriptionMode="tooltip" grouped={true} />
        <SoundPicker
          label={t("settings.debug.soundTheme.label")}
          description={t("settings.debug.soundTheme.description")}
        />
        {/* Cancel shortcut is disabled on Linux due to instability with dynamic shortcut registration */}
        {!isLinux && (
          <ShortcutInput
            shortcutId="cancel"
            grouped={true}
          />
        )}
      </SettingsGroup>
    </div>
  );
};
