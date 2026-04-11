import { listen } from "@tauri-apps/api/event";
import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pause, Play } from "lucide-react";
import { SpeakingIcon } from "../components/icons";
import "./SpeakingOverlay.css";
import { commands } from "@/bindings";
import i18n, { syncLanguageFromSettings } from "@/i18n";
import { getLanguageDirection } from "@/lib/utils/rtl";

type OverlayState = "processing" | "speaking";

interface OverlayPayload {
  state: OverlayState;
  text?: string;
}

const SpeakingOverlay: React.FC = () => {
  const { t } = useTranslation();
  const [isVisible, setIsVisible] = useState(false);
  const [state, setState] = useState<OverlayState>("processing");
  const [speakingPaused, setSpeakingPaused] = useState(false);
  const [isTogglingPause, setIsTogglingPause] = useState(false);
  const [showCloseButton, setShowCloseButton] = useState<boolean>(true);
  const [spokenText, setSpokenText] = useState<string>("");
  // Suppresses size transitions when the overlay reappears after being
  // hidden, so it snaps to the correct size instantly instead of animating
  // from the previous state's dimensions.
  const [noTransition, setNoTransition] = useState(true);
  const wasVisibleRef = useRef(false);
  const overlayRef = useRef<HTMLDivElement>(null);
  const direction = getLanguageDirection(i18n.language);

  // Observe the overlay element's rendered height and tell the backend to
  // resize the native window to match. This keeps the window tightly fitted
  // to the CSS content (which uses height: auto) so the bottom edge stays
  // anchored correctly regardless of text length.
  const lastReportedHeightRef = useRef(0);
  useEffect(() => {
    const el = overlayRef.current;
    if (!el) return;

    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const height =
          entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
        // Skip zero-height (unmounted) and sub-pixel noise to avoid
        // redundant IPC calls during CSS transitions.
        if (
          height > 0 &&
          Math.abs(height - lastReportedHeightRef.current) >= 1
        ) {
          lastReportedHeightRef.current = height;
          commands.resizeOverlay(height);
        }
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const setupEventListeners = async () => {
      const unlistenShow = await listen("show-overlay", async (event) => {
        await syncLanguageFromSettings();
        const settings = await commands.getAppSettings();
        if (settings && settings.status === "ok") {
          setShowCloseButton(settings.data.show_close_button ?? true);
        }

        const payload = event.payload as OverlayPayload;

        if (payload.state === "speaking") {
          if (payload.text) {
            setSpokenText(payload.text);
          }
        } else {
          setSpokenText("");
          setSpeakingPaused(false);
          setIsTogglingPause(false);
        }

        setState(payload.state);

        if (!wasVisibleRef.current) {
          setNoTransition(true);
          setIsVisible(true);
          wasVisibleRef.current = true;
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              setNoTransition(false);
            });
          });
        } else {
          setIsVisible(true);
        }
      });

      const unlistenHide = await listen("hide-overlay", () => {
        setIsVisible(false);
        wasVisibleRef.current = false;
      });

      const unlistenText = await listen<string>("overlay-text", (event) => {
        setSpokenText(event.payload);
      });

      const unlistenPauseState = await listen<boolean>(
        "tts-pause-state",
        (event) => {
          setSpeakingPaused(event.payload);
        },
      );

      return () => {
        unlistenShow();
        unlistenHide();
        unlistenText();
        unlistenPauseState();
      };
    };

    setupEventListeners();
  }, []);

  const togglePause = async () => {
    if (isTogglingPause) {
      return;
    }
    setIsTogglingPause(true);
    try {
      const result = await commands.toggleTtsPause();
      if (result.status === "ok") {
        setSpeakingPaused(result.data);
      }
    } finally {
      setIsTogglingPause(false);
    }
  };

  // Both content layers are always rendered. The outer div's CSS class
  // (processing-state / speaking-state) drives the size transition.
  // Each content layer cross-fades via its own opacity transition — the
  // processing content fades out while the box expands, and the speaking
  // content fades in once the box has reached its full width.
  const isSpeaking = state === "speaking";

  return (
    <div
      ref={overlayRef}
      dir={direction}
      className={`speaking-overlay ${state}-state ${isVisible ? "fade-in" : ""} ${speakingPaused ? "paused" : ""} ${noTransition ? "no-transition" : ""}`}
    >
      {showCloseButton && (
        <button
          type="button"
          className="mac-close-button"
          onClick={() => commands.cancelOperation()}
          title={t("overlay.close", { defaultValue: "Close" })}
          aria-label="Close"
        />
      )}

      <div
        className={`overlay-layer processing-layer ${!isSpeaking ? "active" : ""}`}
      >
        <div className="overlay-left">
          <SpeakingIcon width={24} height={24} />
        </div>
        <div className="overlay-middle">
          <div className="status-text">
            {t("overlay.processing", { defaultValue: "Processing..." })}
          </div>
        </div>
      </div>

      <div
        className={`overlay-layer speaking-layer ${isSpeaking ? "active" : ""}`}
      >
        <div className="speaking-content">
          <div className="speaking-text-section">
            <span className="speaking-text">{spokenText}</span>
          </div>
          <div className="speaking-controls">
            <div className="waveform-bars">
              <div className="waveform-bar" />
              <div className="waveform-bar" />
              <div className="waveform-bar" />
              <div className="waveform-bar" />
              <div className="waveform-bar" />
            </div>
            <button
              type="button"
              className="speaking-toggle-button"
              onClick={togglePause}
              disabled={isTogglingPause}
              title={speakingPaused ? "Resume" : "Pause"}
              aria-label={speakingPaused ? "Resume playback" : "Pause playback"}
            >
              {speakingPaused ? (
                <Play size={16} fill="currentColor" />
              ) : (
                <Pause size={16} fill="currentColor" />
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default SpeakingOverlay;
