import { expect, test } from '@playwright/test';

test.describe('hive task domain', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/hive');
    await expect(page.getByTestId('hive-domain')).toBeVisible();
  });

  test('switches all coordination tabs with the keyboard', async ({ page }) => {
    const threads = page.getByRole('tab', { name: 'スレッド' });
    await expect(threads).toHaveAttribute('data-dioxus-id', /.+/);
    await threads.focus();
    await page.keyboard.press('Enter');

    await expect(threads).toHaveAttribute('aria-selected', 'true');
    await expect(page.getByText('会話はまだありません。')).toBeVisible();
  });

  test('keeps the 320px viewport free of root overflow', async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 720 });

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > 320);
    expect(overflow).toBe(false);
  });

  test('shows task columns with Japanese labels', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'タスクを作成' })).toBeVisible();
    await expect(page.getByLabel('タスク名')).toBeEditable();
    await expect(page.getByRole('heading', { name: 'TODO' })).toBeVisible();
    await expect(page.getByRole('heading', { name: '進行中' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'ブロック中' })).toBeVisible();
    await expect(page.getByRole('heading', { name: '完了' })).toBeVisible();
  });

  test('exposes the new-thread composer', async ({ page }) => {
    const threads = page.getByRole('tab', { name: 'スレッド' });
    await expect(threads).toHaveAttribute('data-dioxus-id', /.+/);
    await threads.click();

    await expect(page.getByRole('heading', { name: '新しいスレッド' })).toBeVisible();
    await expect(page.getByLabel('件名')).toBeEditable();
    await expect(page.getByLabel('本文')).toBeEditable();
  });
});
