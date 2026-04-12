import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { type ClipboardHandling } from "@/bindings";

interface SelectionClipboardHandlingProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const SelectionClipboardHandlingSetting: React.FC<SelectionClipboardHandlingProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const selectedHandling =
      (getSetting("clipboard_handling") as ClipboardHandling) ?? "dont_modify";

    const options = [
      {
        value: "dont_modify",
        label: t(
          "settings.advanced.clipboardHandling.options.dontModify",
          "Don't Modify Clipboard",
        ),
      },
      {
        value: "copy_to_clipboard",
        label: t(
          "settings.advanced.clipboardHandling.options.copyToClipboard",
          "Copy Selection To Clipboard",
        ),
      },
    ];

    return (
      <SettingContainer
        title={t(
          "settings.advanced.clipboardHandling.title",
          "Clipboard Handling",
        )}
        description={t(
          "settings.advanced.clipboardHandling.description",
          "Choose whether Parrot restores your previous clipboard after capturing the selection or leaves the captured text in the clipboard.",
        )}
        descriptionMode={descriptionMode}
        grouped={grouped}
        tooltipPosition="bottom"
      >
        <Dropdown
          options={options}
          selectedValue={selectedHandling}
          onSelect={(value) =>
            updateSetting("clipboard_handling", value as ClipboardHandling)
          }
          disabled={isUpdating("clipboard_handling")}
        />
      </SettingContainer>
    );
  });
