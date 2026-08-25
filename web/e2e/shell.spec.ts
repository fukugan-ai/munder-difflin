import { expect, test, type Page } from "@playwright/test";

const baseUrl = process.env.MD_WEB_BASE_URL ?? "http://127.0.0.1:5080";

function collectPageErrors(page: Page) {
  const errors: string[] = [];

  page.on("pageerror", (error) => errors.push(`pageerror: ${error.message}`));
  page.on("console", (message) => {
    if (message.type() === "error") {
      errors.push(`console.error: ${message.text()}`);
    }
  });

  return errors;
}

async function openShell(page: Page) {
  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });

  await expect(page).toHaveTitle("Munder Difflin");
  await expect(page.locator(".office-titlebar")).toBeVisible();
  await expect(page.getByLabel("Munder Difflin")).toBeVisible();
  await expect(page.getByRole("application", { name: "AIチームのオフィスフロア" })).toBeVisible();
}

test("ローカルWebシェルがXなしで表示され、キーボードフォーカスが見える", async ({ page }) => {
  const errors = collectPageErrors(page);

  await openShell(page);
  await page.keyboard.press("Tab");

  const focus = await page.evaluate(() => {
    const element = document.activeElement;
    if (!(element instanceof HTMLElement)) {
      return { tagName: "", focusVisible: false, hasIndicator: false };
    }

    const style = getComputedStyle(element);
    const hasOutline = style.outlineStyle !== "none" && Number.parseFloat(style.outlineWidth) > 0;
    const hasShadow = style.boxShadow !== "none";

    return {
      tagName: element.tagName,
      focusVisible: element.matches(":focus-visible"),
      hasIndicator: hasOutline || hasShadow,
    };
  });

  expect(focus.tagName).not.toBe("BODY");
  expect(focus.focusVisible).toBe(true);
  expect(focus.hasIndicator).toBe(true);
  expect(errors).toEqual([]);
});

for (const viewport of [
  { name: "desktop", width: 1280, height: 800 },
  { name: "mobile", width: 390, height: 844 },
]) {
  test(`${viewport.name}で横方向にはみ出さない`, async ({ page }) => {
    const errors = collectPageErrors(page);
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await openShell(page);

    const overflow = await page.evaluate(() => ({
      document: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      body: document.body.scrollWidth - document.body.clientWidth,
    }));

    expect(overflow.document).toBeLessThanOrEqual(0);
    expect(overflow.body).toBeLessThanOrEqual(0);
    expect(errors).toEqual([]);
  });
}

test("システムのライト・ダーク設定を反映する", async ({ page }) => {
  const errors = collectPageErrors(page);

  await page.emulateMedia({ colorScheme: "light" });
  await openShell(page);
  await expect.poll(() => page.evaluate(() => matchMedia("(prefers-color-scheme: light)").matches)).toBe(true);
  const lightBackground = await page.locator(".office-domain").evaluate((element) => getComputedStyle(element).backgroundColor);

  await page.emulateMedia({ colorScheme: "dark" });
  await expect.poll(() => page.evaluate(() => matchMedia("(prefers-color-scheme: dark)").matches)).toBe(true);
  const darkBackground = await page.locator(".office-domain").evaluate((element) => getComputedStyle(element).backgroundColor);

  expect(darkBackground).not.toBe(lightBackground);
  expect(errors).toEqual([]);
});

test("日本語fontとproduct versionを同一originから読み込む", async ({ page, request }) => {
  const errors = collectPageErrors(page);
  await openShell(page);

  const font = await page.evaluate(async () => {
    await document.fonts.ready;
    const family = getComputedStyle(document.body).fontFamily;
    return {
      family,
      loaded: document.fonts.check('16px "Noto Sans JP"', "日本語の表示確認"),
    };
  });
  expect(font.family.split(",")[0]?.replaceAll('"', "").trim()).toBe("Noto Sans JP");
  expect(font.loaded).toBe(true);

  const response = await request.get(`${baseUrl}/api/health`);
  expect(response.ok()).toBe(true);
  expect((await response.json()).app_version).toBe("0.4.5");
  expect(errors).toEqual([]);
});
