import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { LanguageSelector } from "../LanguageSelector";
import { KokoroVoiceSelector } from "../KokoroVoiceSelector";
import { useModelStore } from "../../../stores/modelStore";
import type { ModelInfo } from "@/bindings";

export const ModelSettingsCard: React.FC = () => {
  const { t } = useTranslation();
  const { currentModel, models } = useModelStore();

  const currentModelInfo = models.find((m: ModelInfo) => m.id === currentModel);

  const supportsLanguageSelection = currentModelInfo?.engine_type === "Kokoro";

  // Don't render anything if no model is selected or no settings available
  if (!currentModel || !currentModelInfo || !supportsLanguageSelection) {
    return null;
  }

  return (
    <SettingsGroup
      title={t("settings.modelSettings.title", {
        model: currentModelInfo.name,
      })}
    >
      <LanguageSelector
        descriptionMode="tooltip"
        grouped={true}
        supportedLanguages={currentModelInfo.supported_languages}
      />
      {currentModelInfo?.engine_type === "Kokoro" && (
        <KokoroVoiceSelector descriptionMode="tooltip" grouped={true} />
      )}
    </SettingsGroup>
  );
};
