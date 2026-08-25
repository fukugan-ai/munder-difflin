import { expect, test, type Page } from "@playwright/test";

const baseUrl = process.env.MD_WEB_BASE_URL ?? "http://127.0.0.1:5080";

async function openConfigDomain(page: Page) {
  await page.goto(`${baseUrl}/settings`, { waitUntil: "domcontentloaded" });
  await expect(page.locator("#config-onboarding")).toBeVisible();
}

test("設定画面はsecret値をDOMへ返さず、Web版N/Aを明示する", async ({ page }) => {
  await openConfigDomain(page);

  await expect(page.getByRole("heading", { name: "設定", level: 1 })).toBeVisible();
  await expect(page.getByText("ネイティブ自動更新", { exact: true })).toBeVisible();
  await expect(page.getByText("N/A", { exact: true }).first()).toBeVisible();
  await expect(
    page.locator(".toggle-row").filter({ hasText: "複数フロア" }).getByRole("switch"),
  ).toBeDisabled();
  await expect(page.getByRole("button", { name: "新しいフロアを開く" })).toBeDisabled();

  const body = await page.locator("body").innerText();
  expect(body).not.toMatch(/xoxb-|sk-[A-Za-z0-9]/);
});

for (const width of [320, 375, 414, 768]) {
  test(`設定画面は${width}pxで横にはみ出さない`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await openConfigDomain(page);

    const overflow = await page.evaluate(() => ({
      document: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      body: document.body.scrollWidth - document.body.clientWidth,
    }));

    expect(overflow.document).toBeLessThanOrEqual(0);
    expect(overflow.body).toBeLessThanOrEqual(0);
  });
}

test("設定の操作要素へキーボードで到達できる", async ({ page }) => {
  await openConfigDomain(page);

  await page.getByRole("button", { name: "再確認" }).focus();
  await expect(page.getByRole("button", { name: "再確認" })).toBeFocused();
  await expect(page.getByRole("button", { name: "再確認" })).toHaveCSS("outline-style", /solid|dashed|double/);
});
