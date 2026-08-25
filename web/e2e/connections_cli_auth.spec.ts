import { expect, test } from '@playwright/test';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';

const stateDir = process.env.MD_TEST_CLI_AUTH_STATE_DIR;
const expectNotInstalled = process.env.MD_EXPECT_CLAUDE_NOT_INSTALLED === 'true';

test.skip(!stateDir, 'fake CLI auth environment is required');

async function setMode(provider: 'codex' | 'claude', mode: string) {
  if (!stateDir) throw new Error('MD_TEST_CLI_AUTH_STATE_DIR is required');
  await mkdir(stateDir, { recursive: true, mode: 0o700 });
  await writeFile(path.join(stateDir, `${provider}.mode`), `${mode}\n`, { mode: 0o600 });
}

async function clearState() {
  if (!stateDir) throw new Error('MD_TEST_CLI_AUTH_STATE_DIR is required');
  await rm(stateDir, { recursive: true, force: true });
  await mkdir(stateDir, { recursive: true, mode: 0o700 });
}

test.beforeAll(clearState);

test('fake CLI browser auth states and client-only official links', async ({ page, context }) => {
  test.skip(expectNotInstalled, 'not-installed phase runs separately');
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await context.route('https://auth.openai.com/**', (route) =>
    route.fulfill({ status: 200, contentType: 'text/html', body: '<h1>fake OpenAI auth</h1>' }),
  );
  await context.route('https://claude.ai/**', (route) =>
    route.fulfill({ status: 200, contentType: 'text/html', body: '<h1>fake Claude auth</h1>' }),
  );
  await page.goto('/connections');

  const card = page.getByTestId('cli-auth-card');
  const codex = card.locator('[data-provider="Codex"]');
  const claude = card.locator('[data-provider="Claude Code"]');
  await expect(card).toBeVisible();
  await expect(codex.getByRole('button', { name: '接続', exact: true })).toBeVisible();
  await expect(claude.getByRole('button', { name: '接続', exact: true })).toBeVisible();

  await setMode('codex', 'slow');
  const codexStart = codex.getByRole('button', { name: '接続', exact: true });
  await codexStart.focus();
  await page.keyboard.press('Enter');
  await expect(codex.getByRole('link', { name: 'サインインを開く' })).toBeVisible();
  await codex.getByRole('button', { name: 'キャンセル' }).click();
  await expect(codex.getByRole('button', { name: '再試行' })).toBeFocused({ timeout: 3_000 });

  await setMode('codex', 'error');
  await codex.getByRole('button', { name: '再試行' }).focus();
  await page.keyboard.press('Enter');
  await expect(codex.getByRole('button', { name: '再試行' })).toBeVisible({ timeout: 5_000 });
  await expect(codex.getByRole('alert')).toBeVisible();

  await setMode('codex', 'success');
  await codex.getByRole('button', { name: '再試行' }).click();
  const codexLink = codex.getByRole('link', { name: 'サインインを開く' });
  await expect(codexLink).toHaveAttribute('href', 'https://auth.openai.com/device');
  await expect(codexLink).toHaveAttribute('target', '_blank');
  await expect(codexLink).toHaveAttribute('rel', /noopener/);
  await expect(codex.getByText('ABCD-EFGH', { exact: true })).toBeVisible();
  await codex.getByRole('button', { name: 'コードをコピー' }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe('ABCD-EFGH');
  const [openAiPage] = await Promise.all([context.waitForEvent('page'), codexLink.click()]);
  await openAiPage.waitForLoadState();
  await expect(openAiPage.getByRole('heading', { name: 'fake OpenAI auth' })).toBeVisible();
  await openAiPage.close();
  await expect(codex.getByRole('button', { name: '接続済み' })).toBeVisible({ timeout: 8_000 });

  await setMode('claude', 'success');
  await claude.getByRole('button', { name: '接続', exact: true }).click();
  const claudeLink = claude.getByRole('link', { name: 'サインインを開く' });
  await expect(claudeLink).toHaveAttribute('href', 'https://claude.ai/oauth/authorize');
  await expect(claudeLink).toHaveAttribute('target', '_blank');
  await expect(claudeLink).toHaveAttribute('rel', /noopener/);
  const [claudePage] = await Promise.all([context.waitForEvent('page'), claudeLink.click()]);
  await claudePage.waitForLoadState();
  await expect(claudePage.getByRole('heading', { name: 'fake Claude auth' })).toBeVisible();
  await claudePage.close();
  await expect(claude.getByRole('button', { name: '接続済み' })).toBeVisible({ timeout: 8_000 });

  for (const width of [320, 375, 414, 768]) {
    await page.setViewportSize({ width, height: 900 });
    const dimensions = await page.evaluate(() => ({
      client: document.documentElement.clientWidth,
      scroll: document.documentElement.scrollWidth,
      controls: [...document.querySelectorAll('[data-testid=cli-auth-card] button, [data-testid=cli-auth-card] a')]
        .map((element) => ({
          height: element.getBoundingClientRect().height,
          clientWidth: (element as HTMLElement).clientWidth,
          scrollWidth: (element as HTMLElement).scrollWidth,
          whiteSpace: getComputedStyle(element).whiteSpace,
        })),
    }));
    expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.client);
    expect(dimensions.controls.every(({ height }) => height >= 44)).toBe(true);
    expect(dimensions.controls.every(({ clientWidth, scrollWidth }) => scrollWidth <= clientWidth)).toBe(true);
    expect(dimensions.controls.every(({ whiteSpace }) => whiteSpace === 'nowrap')).toBe(true);
  }
});

test('missing fake Claude binary is explicit NotInstalled', async ({ page }) => {
  test.skip(!expectNotInstalled, 'full fake provider phase runs separately');
  await page.goto('/connections');
  const claude = page.getByTestId('cli-auth-card').locator('[data-provider="Claude Code"]');
  await expect(claude.getByRole('button', { name: 'CLI未検出' })).toBeDisabled();
  await expect(claude.getByText('CLIが見つかりません', { exact: true })).toBeVisible();
});
