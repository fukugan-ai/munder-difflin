import { expect, test, type Page } from "@playwright/test";

const baseUrl = process.env.MD_WEB_BASE_URL ?? "http://127.0.0.1:5080";

async function openOffice(page: Page) {
  await page.goto(baseUrl, { waitUntil: "domcontentloaded" });
  await expect(page.locator(".office-domain")).toBeVisible();
  await expect(page.getByRole("application", { name: "AIチームのオフィスフロア" })).toBeVisible();
}

test("元のデスクトップ構造を表示する", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await openOffice(page);

  await expect(page.locator(".office-titlebar")).toBeVisible();
  await expect(page.locator(".office-stage")).toBeVisible();
  await expect(page.locator(".office-detail")).toBeVisible();
  await expect(page.getByRole("region", { name: "エージェント一覧" })).toBeVisible();
  const logo = page.locator(".office-brand__mark");
  await expect(logo).toBeVisible();
  expect(await logo.evaluate((image: HTMLImageElement) => image.naturalWidth)).toBeGreaterThan(0);
});

test("追加モーダルは4手順と実キャラクターを保持する", async ({ page }) => {
  await openOffice(page);
  await page.getByRole("button", { name: "エージェントを追加", exact: true }).click();

  const dialog = page.getByRole("dialog", { name: "エージェントを追加" });
  await expect(dialog).toBeVisible();
  for (const step of ["1 基本情報", "2 作業場所", "3 エンジン", "4 役割設定"]) {
    await expect(dialog.getByRole("button", { name: new RegExp(step) })).toBeVisible();
  }
  await expect(dialog.getByLabel("名前")).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(dialog.getByRole("button", { name: "起動" })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(dialog.getByLabel("名前")).toBeFocused();

  await dialog.getByRole("button", { name: "Andy", exact: true }).click();
  await expect(dialog.getByLabel("名前")).toHaveValue("Andy");
  await expect(dialog.locator('[data-office-portrait="andy"]')).toBeVisible();

  await dialog.getByRole("button", { name: "AIで生成…" }).click();
  await expect(dialog.getByLabel("AI用プロンプト")).toHaveValue(/munder-difflin\/hire@1/);

  await dialog.locator('input[type="file"]').setInputFiles({
    name: "darryl.json",
    mimeType: "application/json",
    buffer: Buffer.from(JSON.stringify({
      spec: "munder-difflin/hire@1",
      name: "Darryl",
      description: "倉庫と運用を担当",
      goal: "運用タスクを完了する",
      provider: "codex",
      model: "gpt-5.6",
      character: "darryl",
      accent: "mint",
      isolate: false,
    })),
  });
  await expect(dialog.getByLabel("名前")).toHaveValue("Darryl");
  await expect(dialog.getByRole("status")).toContainText("darryl.json を読み込みました");

  const cancel = dialog.getByRole("button", { name: "キャンセル" });
  await expect(cancel).toBeInViewport();
  await expect(cancel).toBeEnabled();
  await cancel.click();
  await page.getByRole("button", { name: "エージェントを追加", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "エージェントを追加" }).getByLabel("名前")).toHaveValue("");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "エージェントを追加" })).toBeHidden();
});

test("オフィス島はElectron bridgeを公開しない", async ({ page }) => {
  await openOffice(page);

  const floor = page.locator("[data-office-island]");
  await expect(floor).toHaveAttribute("data-renderer", "pixi");
  await expect(floor).toHaveAttribute("data-runtime-marker", "pixi-tmj-ready");
  await expect(floor).toHaveAttribute("data-pixi-state", "ready");
  await expect(floor).toHaveAttribute("data-map-loaded", /^(office|brooklyn99)$/);
  await expect(floor).not.toHaveAttribute("data-load-error", /.+/);
  await expect(floor).toHaveAttribute("data-action-bridge", "ready");
  expect(await page.evaluate(() => "cth" in window)).toBe(false);
});

test("Darrylのportraitとwalk artはJimと異なる", async ({ page }) => {
  await openOffice(page);
  await expect(page.locator("[data-office-island]")).toHaveAttribute("data-art-state", "ready");
  const distinct = await page.evaluate(() => {
    const art = (globalThis as typeof globalThis & { OfficePortraitArt?: {
      paintPortrait: (context: CanvasRenderingContext2D, name: string, scale: number) => void;
      sceneFrameBufs: (name: string) => { front: Uint8ClampedArray[] };
    } }).OfficePortraitArt;
    if (!art) return false;
    const render = (name: string) => {
      const canvas = document.createElement("canvas");
      canvas.width = 36;
      canvas.height = 56;
      art.paintPortrait(canvas.getContext("2d")!, name, 2);
      return canvas.toDataURL();
    };
    return render("darryl") !== render("jim")
      && String(art.sceneFrameBufs("darryl").front[0]) !== String(art.sceneFrameBufs("jim").front[0]);
  });
  expect(distinct).toBe(true);
});

for (const width of [320, 375, 414, 768]) {
  test(`${width}pxで横方向にはみ出さない`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await openOffice(page);

    const overflow = await page.evaluate(() => ({
      document: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      body: document.body.scrollWidth - document.body.clientWidth,
    }));
    expect(overflow.document).toBeLessThanOrEqual(0);
    expect(overflow.body).toBeLessThanOrEqual(0);
  });
}

test("テーマ、設定、集中モードにキーボードで到達できる", async ({ page }) => {
  await openOffice(page);

  for (const name of ["表示テーマを切り替える", "設定を開く", "集中モードを切り替える"]) {
    const button = page.getByRole("button", { name });
    await button.focus();
    await expect(button).toBeFocused();
    const outline = await button.evaluate((element) => getComputedStyle(element).outlineStyle);
    expect(outline).not.toBe("none");
  }
});

test("選択詳細と集中モードの10タブを保持しEscapeで戻る", async ({ page }) => {
  await openOffice(page);
  const floor = page.locator("[data-office-island]");
  const agentIds = await floor.locator("[data-office-agent]").evaluateAll((agents) =>
    agents.map((agent) => (agent as HTMLElement).dataset.id).filter(Boolean) as string[]
  );
  expect(agentIds.length).toBeGreaterThan(0);
  await floor.evaluate((element, agentId) => {
    element.dispatchEvent(new CustomEvent("office-ui-action", {
      bubbles: true,
      detail: { type: "select_agent", data: { agent_id: agentId } },
    }));
  }, agentIds[0]);
  await expect(page.locator(".office-domain")).toHaveAttribute("data-selected-agent-id", agentIds[0]);
  await expect(page.locator(".agent-detail-host")).toHaveAttribute("data-agent-detail-id", agentIds[0]);

  await page.getByRole("button", { name: "集中モードを切り替える" }).click();
  await expect(page.locator(".office-domain")).toHaveAttribute("data-focus-mode", "true");
  const tabs = page.getByRole("navigation", { name: "コマンドセンター" }).getByRole("button");
  await expect(tabs).toHaveText([
    "ターミナル", "モニター", "タスク", "質問", "トリガー",
    "メモリ", "グラフ", "アクティビティ", "コマンド", "ワーカー",
  ]);
  await page.keyboard.press("Escape");
  await expect(page.locator(".office-domain")).toHaveAttribute("data-focus-mode", "false");
});

test("agent依存の全タブは選択AからBへ同じcontextで再生成される", async ({ page }) => {
  await openOffice(page);
  const floor = page.locator("[data-office-island]");
  const agentIds = await floor.locator("[data-office-agent]").evaluateAll((agents) =>
    agents.map((agent) => (agent as HTMLElement).dataset.id).filter(Boolean) as string[]
  );
  expect(agentIds.length).toBeGreaterThanOrEqual(2);
  const [agentA, agentB] = agentIds;

  await page.getByRole("button", { name: "集中モードを切り替える" }).click();
  await expect(page.locator(".office-domain")).toHaveAttribute("data-focus-mode", "true");

  for (const tab of ["ターミナル", "モニター", "タスク", "質問", "メモリ", "コマンド", "ワーカー"]) {
    await page.getByRole("navigation", { name: "コマンドセンター" })
      .getByRole("button", { name: tab, exact: true }).click();
    await page.locator(`.agent-card[data-agent-id="${agentA}"]`).click();
    await expect(page.locator(".office-command-center")).toHaveAttribute("data-agent-id", agentA);
    await expect(page.locator(".agent-detail-host__content")).toHaveAttribute("data-agent-content-id", agentA);

    await page.locator(`.agent-card[data-agent-id="${agentB}"]`).click();
    await expect(page.locator(".office-command-center")).toHaveAttribute("data-agent-id", agentB);
    await expect(page.locator(".agent-detail-host__content")).toHaveAttribute("data-agent-content-id", agentB);
  }
  await page.keyboard.press("Escape");
  await expect(page.locator(".office-domain")).toHaveAttribute("data-focus-mode", "false");
});

test("フロアのDTOアクションはRust callbackへ届く", async ({ page }) => {
  await openOffice(page);
  const floor = page.locator("[data-office-island]");
  await expect(floor).toHaveAttribute("data-action-bridge", "ready");
  const firstAgent = await floor.locator("[data-office-agent]").first().getAttribute("data-id");
  expect(firstAgent).toBeTruthy();
  await floor.evaluate((element, agentId) => {
    element.dispatchEvent(new CustomEvent("office-ui-action", {
      bubbles: true,
      detail: { type: "select_agent", data: { agent_id: agentId } },
    }));
  }, firstAgent);
  await expect(floor).toHaveAttribute("data-selected-agent", firstAgent!);

  await floor.evaluate((element) => {
    element.dispatchEvent(new CustomEvent("office-ui-action", {
      bubbles: true,
      detail: { type: "open_tasks" },
    }));
  });
  await expect(page.getByRole("navigation", { name: "コマンドセンター" })
    .getByRole("button", { name: "タスク", exact: true })).toHaveAttribute("aria-selected", "true");

  await floor.evaluate((element) => {
    element.dispatchEvent(new CustomEvent("office-ui-action", {
      bubbles: true,
      detail: { type: "open_human_questions" },
    }));
  });
  await expect(page.getByRole("navigation", { name: "コマンドセンター" })
    .getByRole("button", { name: "質問", exact: true })).toHaveAttribute("aria-selected", "true");

  await floor.focus();
  await expect(floor).toBeFocused();
  await page.keyboard.press("t");
  await expect(page.getByRole("navigation", { name: "コマンドセンター" })
    .getByRole("button", { name: "タスク", exact: true })).toHaveAttribute("aria-selected", "true");
});
