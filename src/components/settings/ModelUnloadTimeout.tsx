import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";
import { type ModelUnloadTimeout } from "@/bindings";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";

interface ModelUnloadTimeoutProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

export const ModelUnloadTimeoutSetting: React.FC<ModelUnloadTimeoutProps> = ({
  descriptionMode = "inline",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { settings, getSetting, updateSetting, isUpdating } = useSettings();

  const timeoutOptions = [
    {
      value: "never" as ModelUnloadTimeout,
      label: t("settings.advanced.modelUnload.options.never"),
    },
    {
      value: "immediately" as ModelUnloadTimeout,
      label: t("settings.advanced.modelUnload.options.immediately"),
    },
    {
      value: "min_2" as ModelUnloadTimeout,
      label: t("settings.advanced.modelUnload.options.min2"),
    },
    {
      value: "min_5" as ModelUnloadTimeout,
      label: t("settings.advanced.modelUnload.options.min5"),
    },
    {
      value: "min_10" as ModelUnloadTimeout,
      label: t("settings.advanced.modelUnload.options.min10"),
    },
    {
      value: "min_15" as ModelUnloadTimeout,
      label: t("settings.advanced.modelUnload.options.min15"),
    },
    {
      value: "hour_1" as ModelUnloadTimeout,
      label: t("settings.advanced.modelUnload.options.hour1"),
    },
  ];

  const debugTimeoutOptions = [
    ...timeoutOptions,
    {
      value: "sec_5" as ModelUnloadTimeout,
      label: t("settings.advanced.modelUnload.options.sec5"),
    },
  ];

  const currentValue = getSetting("model_unload_timeout") ?? "never";

  const options = useMemo(() => {
    return settings?.debug_mode === true ? debugTimeoutOptions : timeoutOptions;
  }, [settings]);

  return (
    <SettingContainer
      title={t("settings.advanced.modelUnload.title")}
      description={t("settings.advanced.modelUnload.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <Dropdown
        options={options}
        selectedValue={currentValue}
        onSelect={(value) =>
          updateSetting("model_unload_timeout", value as ModelUnloadTimeout)
        }
        disabled={isUpdating("model_unload_timeout")}
      />
    </SettingContainer>
  );
};
