import { expect, test, type Page } from "@playwright/test";

const baseUrl = process.env.MD_WEB_BASE_URL ?? "http://127.0.0.1:8080";

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
  await expect(page.getByText("MUNDER DIFFLIN", { exact: true })).toBeVisible();
  await expect(page.getByText("ローカルWeb版", { exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "オフィス", level: 1 })).toBeVisible();

  const serverStatus = page.locator(".health-list__row").filter({ hasText: "Webサーバー" });
  const postgresStatus = page.locator(".health-list__row").filter({ hasText: "PostgreSQL" });

  await expect(serverStatus).toBeVisible();
  await expect(serverStatus.getByRole("status")).toBeVisible();
  await expect(postgresStatus).toBeVisible();
  await expect(postgresStatus.getByRole("status")).toBeVisible();
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
  const lightBackground = await page.locator(".app-shell").evaluate((element) => getComputedStyle(element).backgroundColor);

  await page.emulateMedia({ colorScheme: "dark" });
  await expect.poll(() => page.evaluate(() => matchMedia("(prefers-color-scheme: dark)").matches)).toBe(true);
  const darkBackground = await page.locator(".app-shell").evaluate((element) => getComputedStyle(element).backgroundColor);

  expect(darkBackground).not.toBe(lightBackground);
  expect(errors).toEqual([]);
});
