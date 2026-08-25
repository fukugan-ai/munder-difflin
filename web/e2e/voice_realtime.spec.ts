import { expect, test } from "@playwright/test";

const baseUrl = process.env.MD_WEB_BASE_URL ?? "http://127.0.0.1:5080";
const voiceUrl = new URL("/voice", baseUrl).toString();

test.describe("音声パネル", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(voiceUrl, { waitUntil: "domcontentloaded" });
  });

  test("音声機能の説明と二つの操作を表示する", async ({ page }) => {
    const panel = page.getByRole("region", { name: "音声" });
    await expect(panel).toBeVisible();
    await expect(panel.getByRole("heading", { name: "Free Flow" })).toBeVisible();
    await expect(panel.getByRole("heading", { name: "Realtime Michael" })).toBeVisible();
    await expect(panel.getByRole("button", { name: "録音を開始" })).toBeVisible();
    await expect(panel.getByRole("button", { name: "Michaelと話す" })).toBeVisible();
  });

  test("HTTPのLAN接続ではHTTPS警告を表示する", async ({ page }) => {
    const panel = page.getByRole("region", { name: "音声" });
    await expect(panel).toBeVisible();
    await page.evaluate(() => {
      globalThis.dispatchEvent(new CustomEvent("munder:voice-realtime-event", {
        detail: { type: "capabilities", secureContext: false, devices: [] },
      }));
    });

    await expect(panel.getByRole("alert")).toContainText("HTTPS接続が必要です");
  });

  test("Option長押しは文字合成やAltコンビネーションでは録音を開始しない", async ({ page }) => {
    await page.waitForFunction(() => Boolean((globalThis as typeof globalThis & {
      munderVoiceBridge?: unknown;
    }).munderVoiceBridge));
    const calls = await page.evaluate(async () => {
      let getUserMediaCalls = 0;
      Object.defineProperty(navigator.mediaDevices, "getUserMedia", {
        configurable: true,
        value: async () => {
          getUserMediaCalls += 1;
          throw new Error("test microphone");
        },
      });
      const bridge = (globalThis as typeof globalThis & {
        munderVoiceBridge: {
          configureFreeflowShortcut(options: {
            enabled: boolean;
            targetAgentId: string;
            inputDeviceId: string | null;
          }): void;
        };
      }).munderVoiceBridge;
      bridge.configureFreeflowShortcut({
        enabled: true,
        targetAgentId: "worker",
        inputDeviceId: null,
      });
      globalThis.dispatchEvent(new KeyboardEvent("keydown", { key: "Alt", code: "AltLeft" }));
      globalThis.dispatchEvent(new KeyboardEvent("keydown", { key: "Process", isComposing: true }));
      await new Promise((resolve) => setTimeout(resolve, 380));
      globalThis.dispatchEvent(new KeyboardEvent("keyup", { key: "Alt", code: "AltLeft" }));
      return getUserMediaCalls;
    });

    expect(calls).toBe(0);
  });

  test("音声設定はキー値を再表示せずデバイスとTLS状態を表示する", async ({ page }) => {
    await page.getByRole("button", { name: "設定" }).click();
    const settings = page.getByRole("region", { name: "音声設定" });
    await expect(settings.getByLabel(/Groq APIキー/)).toHaveAttribute("type", "password");
    await expect(settings.getByLabel(/OpenAI APIキー/)).toHaveAttribute("type", "password");
    await expect(settings.getByLabel("マイク")).toBeVisible();
    await expect(settings).toContainText("LAN向けHTTPS");
  });

  for (const width of [320, 375, 414, 768]) {
    test(`${width}pxで横方向にはみ出さない`, async ({ page }) => {
      await page.setViewportSize({ width, height: 844 });
      const panel = page.getByRole("region", { name: "音声" });
      await expect(panel).toBeVisible();

      const overflow = await page.evaluate(() =>
        document.documentElement.scrollWidth - document.documentElement.clientWidth,
      );
      expect(overflow).toBeLessThanOrEqual(0);
    });
  }
});
