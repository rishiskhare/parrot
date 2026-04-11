import React from "react";
import { Trans, useTranslation } from "react-i18next";
import { Cog, FlaskConical, History, Info, Cpu, Lightbulb } from "lucide-react";
import ParrotTextLogo from "./icons/ParrotTextLogo";
import ParrotIcon from "./icons/ParrotIcon";
import { useSettings } from "../hooks/useSettings";
import { useOsType } from "../hooks/useOsType";
import { formatKeyCombination, type OSType } from "../lib/utils/keyboard";
import {
  GeneralSettings,
  AdvancedSettings,
  HistorySettings,
  DebugSettings,
  AboutSettings,
  ModelsSettings,
} from "./settings";

const MAC_SYMBOLS: Record<string, string> = {
  command: "⌘",
  cmd: "⌘",
  option: "⌥",
  alt: "⌥",
  shift: "⇧",
  ctrl: "⌃",
  control: "⌃",
  fn: "fn",
};

/** Compact shortcut string using macOS symbols when applicable. */
const formatCompactShortcut = (binding: string, osType: OSType): string => {
  if (!binding) return "";
  if (osType !== "macos") return formatKeyCombination(binding, osType);

  return binding
    .split("+")
    .map((part) => {
      const key = part
        .trim()
        .replace(/_(left|right)$/, "")
        .toLowerCase();
      return MAC_SYMBOLS[key] ?? key.charAt(0).toUpperCase() + key.slice(1);
    })
    .join(" ");
};

export type SidebarSection = keyof typeof SECTIONS_CONFIG;

interface IconProps {
  width?: number | string;
  height?: number | string;
  size?: number | string;
  className?: string;
  [key: string]: any;
}

interface SectionConfig {
  labelKey: string;
  icon: React.ComponentType<IconProps>;
  component: React.ComponentType;
  enabled: (settings: any) => boolean;
}

export const SECTIONS_CONFIG = {
  general: {
    labelKey: "sidebar.general",
    icon: ParrotIcon,
    component: GeneralSettings,
    enabled: () => true,
  },
  models: {
    labelKey: "sidebar.models",
    icon: Cpu,
    component: ModelsSettings,
    enabled: () => true,
  },
  advanced: {
    labelKey: "sidebar.advanced",
    icon: Cog,
    component: AdvancedSettings,
    enabled: () => true,
  },
  history: {
    labelKey: "sidebar.history",
    icon: History,
    component: HistorySettings,
    enabled: () => true,
  },
  debug: {
    labelKey: "sidebar.debug",
    icon: FlaskConical,
    component: DebugSettings,
    enabled: (settings) => settings?.debug_mode ?? false,
  },
  about: {
    labelKey: "sidebar.about",
    icon: Info,
    component: AboutSettings,
    enabled: () => true,
  },
} as const satisfies Record<string, SectionConfig>;

interface SidebarProps {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeSection,
  onSectionChange,
}) => {
  const { t } = useTranslation();
  const { settings, getSetting } = useSettings();
  const osType = useOsType();
  const speakBinding = getSetting("bindings")?.speak?.current_binding;

  const availableSections = Object.entries(SECTIONS_CONFIG)
    .filter(([_, config]) => config.enabled(settings))
    .map(([id, config]) => ({ id: id as SidebarSection, ...config }));

  return (
    <div className="flex flex-col w-40 h-full border-e border-mid-gray/20 items-center px-2">
      <ParrotTextLogo width={120} className="m-4" />
      <div className="flex flex-col w-full items-center gap-1 pt-2 border-t border-mid-gray/20">
        {availableSections.map((section) => {
          const Icon = section.icon;
          const isActive = activeSection === section.id;

          return (
            <div
              key={section.id}
              className={`flex gap-2 items-center p-2 w-full rounded-lg cursor-pointer transition-colors ${
                isActive
                  ? "bg-logo-primary/80"
                  : "hover:bg-mid-gray/20 hover:opacity-100 opacity-85"
              }`}
              onClick={() => onSectionChange(section.id)}
            >
              <Icon width={24} height={24} className="shrink-0" />
              <p
                className="text-sm font-medium truncate"
                title={t(section.labelKey)}
              >
                {t(section.labelKey)}
              </p>
            </div>
          );
        })}
      </div>
      {speakBinding && speakBinding !== "disabled" && (
        <div className="mt-auto w-full border-t border-mid-gray/20 pt-3 pb-3">
          <div className="px-1.5">
            <div className="flex items-center gap-1.5 mb-1.5">
              <Lightbulb className="w-4 h-4 text-yellow-500 shrink-0" />
              <p className="text-sm font-semibold leading-none">
                {t("settings.howToUseTitle")}
              </p>
            </div>
            <div className="text-sm text-text/90 leading-5 space-y-0.5">
              <p>{t("settings.howToUseStep1")}</p>
              <p>
                <Trans
                  i18nKey="settings.howToUseStep2"
                  components={{
                    shortcut: (
                      <kbd className="inline-block whitespace-nowrap px-1.5 py-0.5 text-xs font-medium bg-mid-gray/15 border border-mid-gray/25 rounded shadow-[0_1px_0_0] shadow-mid-gray/20">
                        {formatCompactShortcut(speakBinding, osType)}
                      </kbd>
                    ),
                  }}
                />
              </p>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
