import React from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { type SelectionCaptureMethod } from "@/bindings";

interface SelectionCaptureMethodProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const SelectionCaptureMethodSetting: React.FC<SelectionCaptureMethodProps> =
  React.memo(({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const selectedMethod =
      (getSetting("selection_capture_method") as SelectionCaptureMethod) ??
      "clipboard";

    const isMacOS =
      typeof window !== "undefined" &&
      /mac/i.test(window.navigator.userAgent || "");

    const options = [
      {
        value: "auto",
        label: t("settings.advanced.captureMethod.options.auto", "Auto"),
      },
      ...(isMacOS
        ? [
            {
              value: "accessibility",
              label: t(
                "settings.advanced.captureMethod.options.accessibility",
                "Accessibility",
              ),
            },
          ]
        : []),
      {
        value: "clipboard",
        label: t(
          "settings.advanced.captureMethod.options.clipboard",
          "Clipboard Copy",
        ),
      },
    ];

    return (
      <SettingContainer
        title={t("settings.advanced.captureMethod.title", "Capture Method")}
        description={t(
          "settings.advanced.captureMethod.description",
          "Choose how Parrot captures the selected text before sending it to speech synthesis. Auto prefers direct selection access when available and falls back to clipboard copy.",
        )}
        descriptionMode={descriptionMode}
        grouped={grouped}
        tooltipPosition="bottom"
      >
        <Dropdown
          options={options}
          selectedValue={selectedMethod}
          onSelect={(value) =>
            updateSetting(
              "selection_capture_method",
              value as SelectionCaptureMethod,
            )
          }
          disabled={isUpdating("selection_capture_method")}
        />
      </SettingContainer>
    );
  });
