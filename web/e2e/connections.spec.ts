import { expect, test } from '@playwright/test';

test.describe('外部連携とトリガー', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/connections');
  });

  test('保存済みシークレットを画面へ再表示しない', async ({ page }) => {
    await expect(page.getByTestId('connections-panel')).toBeVisible();
    await expect(page.getByTestId('slack-card')).toBeVisible();
    await expect(page.getByLabel('署名シークレット')).toHaveValue('');
    await expect(page.getByLabel('Botトークン')).toHaveValue('');
    await expect(page.getByLabel('シークレット（変更時だけ入力）')).toHaveValue('');
  });

  test('接続先とカスタムRESTとブローカーを編集できる', async ({ page }) => {
    await expect(page.getByLabel('チャンネルID（任意）')).toBeVisible();
    await expect(page.getByLabel('待受ポート')).toHaveValue('3847');
    await expect(page.getByRole('heading', { name: '外部連携を追加・編集' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'ローカル連携ブローカー' })).toBeVisible();
  });

  test('接続とトリガーをキーボードで切り替えられる', async ({ page }) => {
    const triggerTab = page.getByRole('button', { name: 'トリガー' });
    await triggerTab.focus();
    await page.keyboard.press('Enter');

    await expect(page.getByTestId('triggers-panel')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'スケジュール' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'トリガー履歴' })).toBeVisible();
    await expect(page.getByRole('heading', { name: '条件とメッセージ' })).toBeVisible();
    await expect(page.getByLabel('組織APIキー')).toHaveValue('');

    await page.getByRole('button', { name: '追加' }).click();
    await expect(page.getByRole('heading', { name: 'スケジュールを追加・編集' })).toBeVisible();
  });

  test('320pxで横スクロールしない', async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 800 });

    const dimensions = await page.evaluate(() => ({
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
    }));
    expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
  });

  test('CLI認証カードは既存接続面の中で状態を表示する', async ({ page }) => {
    const card = page.getByTestId('cli-auth-card');
    await expect(card).toBeVisible();
    await expect(card.getByText('Codex', { exact: true })).toBeVisible();
    await expect(card.getByText('Claude Code', { exact: true })).toBeVisible();
    await expect(card.getByRole('button', { name: '状態を更新' })).toBeVisible();
  });

  for (const width of [320, 375, 414, 768]) {
    test(`${width}pxでCLI認証カードが横スクロールしない`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await expect(page.getByTestId('cli-auth-card')).toBeVisible();

      const dimensions = await page.evaluate(() => ({
        clientWidth: document.documentElement.clientWidth,
        scrollWidth: document.documentElement.scrollWidth,
      }));
      expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
    });
  }
});
