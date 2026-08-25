import { expect, test } from "@playwright/test";

test("agent terminal domain supports keyboard-first local workflow", async ({ page }) => {
  await page.goto("/agents");

  const domain = page.locator(".pty-agents");
  await expect(domain).toBeVisible();
  await expect(domain.getByRole("heading", { name: "エージェントとターミナル" })).toBeVisible();

  const launch = domain.getByRole("button", { name: "エージェントを起動" });
  await expect(launch).toBeDisabled();
  await domain.getByLabel("名前").fill("Dev 1");
  await domain.getByLabel("作業フォルダー").fill("/tmp");
  await domain.getByLabel("CLIコマンド").fill("codex");
  await expect(launch).toBeEnabled();
  await launch.focus();
  await expect(launch).toBeFocused();
});

test("agent terminal domain fits a narrow viewport without horizontal scrolling", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 800 });
  await page.goto("/agents");

  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
  expect(overflow).toBe(false);
});

test("bundled xterm handles Japanese width, IME input, paste, and copy", async ({ page }) => {
  await page.goto("/agents");
  await expect.poll(() => page.evaluate(() => typeof globalThis.Terminal)).toBe("function");

  const receipt = await page.evaluate(async () => {
    const host = document.createElement("div");
    host.style.cssText = "position:fixed;left:-10000px;width:640px;height:240px";
    document.body.append(host);
    const terminal = new globalThis.Terminal({ allowProposedApi: true, cols: 20, rows: 5 });
    terminal.open(host);
    const emitted: string[] = [];
    const input = terminal.onData((data: string) => emitted.push(data));
    const textarea = terminal.textarea;
    if (!textarea) throw new Error("xterm helper textarea is missing");

    textarea.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    textarea.value = "日本語";
    textarea.dispatchEvent(new CompositionEvent("compositionupdate", {
      bubbles: true,
      data: "日本語",
    }));
    textarea.dispatchEvent(new CompositionEvent("compositionend", {
      bubbles: true,
      data: "日本語",
    }));
    await new Promise((resolve) => setTimeout(resolve, 0));

    const paste = new Event("paste", { bubbles: true, cancelable: true });
    Object.defineProperty(paste, "clipboardData", {
      value: { getData: () => "貼り付け", setData: () => undefined },
    });
    textarea.dispatchEvent(paste);

    terminal.reset();
    await new Promise<void>((resolve) => terminal.write("日A", resolve));
    const line = terminal.buffer.active.getLine(0);
    const widths = [line?.getCell(0)?.getWidth(), line?.getCell(1)?.getWidth(), line?.getCell(2)?.getWidth()];

    await new Promise<void>((resolve) => terminal.write("\r\nコピー", resolve));
    terminal.select(0, 1, 6);
    let copied = "";
    const copy = new Event("copy", { bubbles: true, cancelable: true });
    Object.defineProperty(copy, "clipboardData", {
      value: { getData: () => "", setData: (_type: string, value: string) => { copied = value; } },
    });
    textarea.dispatchEvent(copy);

    input.dispose();
    terminal.dispose();
    host.remove();
    return { emitted: emitted.join(""), widths, copied };
  });

  expect(receipt.emitted).toContain("日本語");
  expect(receipt.emitted).toContain("貼り付け");
  expect(receipt.widths).toEqual([2, 0, 1]);
  expect(receipt.copied).toContain("コピー");
});
