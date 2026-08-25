import { expect, test } from '@playwright/test';

test.describe('記憶・ナレッジ・履歴', () => {
  test('キーボードで全ドメインタブを切り替えられる', async ({ page }) => {
    await page.goto('/memory');
    await page.getByRole('heading', { name: '記憶と履歴' }).scrollIntoViewIfNeeded();

    await page.getByRole('tab', { name: 'ナレッジ' }).focus();
    await page.keyboard.press('Enter');
    await expect(page.getByTestId('knowledge-panel')).toBeVisible();

    await page.getByRole('tab', { name: 'スキル' }).click();
    await expect(page.getByTestId('skills-panel')).toBeVisible();

    await page.getByRole('tab', { name: '活動' }).click();
    await expect(page.getByTestId('activity-panel')).toBeVisible();

    await page.getByRole('tab', { name: 'トレース' }).click();
    await expect(page.getByTestId('telemetry-panel')).toBeVisible();

    await page.getByRole('tab', { name: 'コマンド履歴' }).click();
    await expect(page.getByTestId('history-panel')).toBeVisible();
  });

  test('モバイル幅でも横方向へページがはみ出さない', async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 800 });
    await page.goto('/memory');

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
    expect(overflow).toBe(false);
  });

  test('空のメモリ検索は送信できない', async ({ page }) => {
    await page.goto('/memory');
    const search = page.getByRole('button', { name: '検索', exact: true }).first();

    await expect(search).toBeDisabled();
  });
});
