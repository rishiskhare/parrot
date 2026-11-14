import { test, expect } from "@playwright/test";
import { mockTauri, invokeCalls } from "./tauri-mock";

const model = (overrides: Record<string, unknown>) => ({
  description: "",
  filename: "",
  url: null,
  size_mb: 115,
  is_downloaded: false,
  is_downloading: false,
  partial_size: 0,
  is_directory: true,
  engine_type: "Kokoro",
  accuracy_score: 0.8,
  speed_score: 0.85,
  is_recommended: false,
  supported_languages: ["en"],
  is_custom: false,
  components: [],
  ...overrides,
});

const models = [
  model({
    id: "kokoro",
    name: "Kokoro-82M",
    description: "Fast and accurate",
    is_downloaded: true,
    is_recommended: true,
  }),
];

async function openModels(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByText("Models", { exact: true }).first().click();
  await expect(page.getByText("Kokoro-82M").first()).toBeVisible();
}

test.describe("Models page", () => {
  test("lists Kokoro without Piper, search or Hugging Face import", async ({
    page,
  }) => {
    await mockTauri(page, {
      get_available_models: models,
      get_current_model: "kokoro",
    });
    await openModels(page);
    await expect(page.getByPlaceholder("Search models")).toHaveCount(0);
    await expect(page.getByTestId("hf-add-form")).toHaveCount(0);
    await expect(page.getByText("Add from Hugging Face")).toHaveCount(0);

    const calls = await invokeCalls(page);
    expect(calls.map((c) => c.cmd)).not.toContain("refresh_model_catalog");
  });
});
