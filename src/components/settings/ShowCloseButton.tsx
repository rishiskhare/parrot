import React from "react";
import { useTranslation } from "react-i18next";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { useSettings } from "../../hooks/useSettings";

interface ShowCloseButtonProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const ShowCloseButton: React.FC<ShowCloseButtonProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const showCloseButtonEnabled = getSetting("show_close_button") ?? true;

    return (
      <ToggleSwitch
        checked={showCloseButtonEnabled}
        onChange={(enabled) => updateSetting("show_close_button", enabled)}
        isUpdating={isUpdating("show_close_button")}
        label={t("settings.general.showCloseButton.label", "Show Close Button")}
        description={t(
          "settings.general.showCloseButton.description",
          "Shows a red close button on the processing and playing screens",
        )}
        descriptionMode={descriptionMode}
        grouped={grouped}
      />
    );
  },
);
