import React, { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { SettingContainer } from "../ui/SettingContainer";
import { ResetButton } from "../ui/ResetButton";
import { Dropdown, type DropdownOption } from "../ui/Dropdown";
import { useSettings } from "../../hooks/useSettings";

interface KokoroVoiceSelectorProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

const VOICE_META: Record<
  string,
  { name: string; lang: string; gender: string }
> = {
  // American English
  af_alloy: { name: "Alloy", lang: "American", gender: "Female" },
  af_aoede: { name: "Aoede", lang: "American", gender: "Female" },
  af_bella: { name: "Bella", lang: "American", gender: "Female" },
  af_heart: { name: "Heart", lang: "American", gender: "Female" },
  af_jessica: { name: "Jessica", lang: "American", gender: "Female" },
  af_kore: { name: "Kore", lang: "American", gender: "Female" },
  af_nicole: { name: "Nicole", lang: "American", gender: "Female" },
  af_nova: { name: "Nova", lang: "American", gender: "Female" },
  af_river: { name: "River", lang: "American", gender: "Female" },
  af_sarah: { name: "Sarah", lang: "American", gender: "Female" },
  af_sky: { name: "Sky", lang: "American", gender: "Female" },
  am_adam: { name: "Adam", lang: "American", gender: "Male" },
  am_echo: { name: "Echo", lang: "American", gender: "Male" },
  am_eric: { name: "Eric", lang: "American", gender: "Male" },
  am_fenrir: { name: "Fenrir", lang: "American", gender: "Male" },
  am_liam: { name: "Liam", lang: "American", gender: "Male" },
  am_michael: { name: "Michael", lang: "American", gender: "Male" },
  am_onyx: { name: "Onyx", lang: "American", gender: "Male" },
  am_puck: { name: "Puck", lang: "American", gender: "Male" },
  am_santa: { name: "Santa", lang: "American", gender: "Male" },
  // British English
  bf_alice: { name: "Alice", lang: "British", gender: "Female" },
  bf_emma: { name: "Emma", lang: "British", gender: "Female" },
  bf_isabella: { name: "Isabella", lang: "British", gender: "Female" },
  bf_lily: { name: "Lily", lang: "British", gender: "Female" },
  bm_daniel: { name: "Daniel", lang: "British", gender: "Male" },
  bm_fable: { name: "Fable", lang: "British", gender: "Male" },
  bm_george: { name: "George", lang: "British", gender: "Male" },
  bm_lewis: { name: "Lewis", lang: "British", gender: "Male" },
  // Japanese
  jf_alpha: { name: "Alpha", lang: "Japanese", gender: "Female" },
  jf_gongitsune: { name: "Gongitsune", lang: "Japanese", gender: "Female" },
  jf_nezumi: { name: "Nezumi", lang: "Japanese", gender: "Female" },
  jf_tebukuro: { name: "Tebukuro", lang: "Japanese", gender: "Female" },
  jm_kumo: { name: "Kumo", lang: "Japanese", gender: "Male" },
  // Mandarin Chinese
  zf_xiaobei: { name: "Xiaobei", lang: "Chinese", gender: "Female" },
  zf_xiaoni: { name: "Xiaoni", lang: "Chinese", gender: "Female" },
  zf_xiaoxiao: { name: "Xiaoxiao", lang: "Chinese", gender: "Female" },
  zf_xiaoyi: { name: "Xiaoyi", lang: "Chinese", gender: "Female" },
  zm_yunjian: { name: "Yunjian", lang: "Chinese", gender: "Male" },
  zm_yunxi: { name: "Yunxi", lang: "Chinese", gender: "Male" },
  zm_yunxia: { name: "Yunxia", lang: "Chinese", gender: "Male" },
  zm_yunyang: { name: "Yunyang", lang: "Chinese", gender: "Male" },
  // Spanish
  ef_dora: { name: "Dora", lang: "Spanish", gender: "Female" },
  em_alex: { name: "Alex", lang: "Spanish", gender: "Male" },
  em_santa: { name: "Santa", lang: "Spanish", gender: "Male" },
  // French
  ff_siwis: { name: "Siwis", lang: "French", gender: "Female" },
  // Hindi
  hf_alpha: { name: "Alpha", lang: "Hindi", gender: "Female" },
  hf_beta: { name: "Beta", lang: "Hindi", gender: "Female" },
  hm_omega: { name: "Omega", lang: "Hindi", gender: "Male" },
  hm_psi: { name: "Psi", lang: "Hindi", gender: "Male" },
  // Italian
  if_sara: { name: "Sara", lang: "Italian", gender: "Female" },
  im_nicola: { name: "Nicola", lang: "Italian", gender: "Male" },
  // Brazilian Portuguese
  pf_dora: { name: "Dora", lang: "Portuguese", gender: "Female" },
  pm_alex: { name: "Alex", lang: "Portuguese", gender: "Male" },
  pm_santa: { name: "Santa", lang: "Portuguese", gender: "Male" },
};

function getVoiceLabel(voiceId: string): string {
  const meta = VOICE_META[voiceId];
  if (meta) {
    return `${meta.name} (${meta.lang} ${meta.gender})`;
  }
  // Fallback: capitalize the name part for unknown voices
  const parts = voiceId.split("_");
  if (parts.length >= 2) {
    const name = parts.slice(1).join("_");
    return name.charAt(0).toUpperCase() + name.slice(1);
  }
  return voiceId;
}

export const KokoroVoiceSelector: React.FC<KokoroVoiceSelectorProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, resetSetting, isUpdating } = useSettings();
  const [voices, setVoices] = useState<string[]>([]);

  const selectedVoice = getSetting("selected_kokoro_voice") ?? null;

  const loadVoices = useCallback(async () => {
    try {
      const result = await commands.getKokoroVoices();
      if (result.status === "ok") {
        setVoices(result.data);
      } else {
        setVoices([]);
      }
    } catch {
      setVoices([]);
    }
  }, []);

  useEffect(() => {
    void loadVoices();
  }, [loadVoices]);

  const dropdownOptions: DropdownOption[] = useMemo(() => {
    return [
      {
        value: "__auto__",
        label: t("settings.modelSettings.kokoroVoice.auto"),
      },
      ...voices.map((voice) => ({
        value: voice,
        label: getVoiceLabel(voice),
      })),
    ];
  }, [voices, t]);

  const handleSelectVoice = async (value: string) => {
    const voiceValue = value === "__auto__" ? null : value;
    await updateSetting("selected_kokoro_voice", voiceValue);
  };

  const handleReset = async () => {
    await resetSetting("selected_kokoro_voice");
  };

  const selectedValue = selectedVoice ?? "__auto__";
  const disabled = isUpdating("selected_kokoro_voice");

  return (
    <SettingContainer
      title={t("settings.modelSettings.kokoroVoice.title")}
      description={t("settings.modelSettings.kokoroVoice.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
    >
      <div className="flex items-center space-x-1">
        <Dropdown
          options={dropdownOptions}
          selectedValue={selectedValue}
          onSelect={handleSelectVoice}
          disabled={disabled}
          onRefresh={loadVoices}
          placeholder={t("settings.modelSettings.kokoroVoice.auto")}
        />
        <ResetButton onClick={handleReset} disabled={disabled} />
      </div>
    </SettingContainer>
  );
};
