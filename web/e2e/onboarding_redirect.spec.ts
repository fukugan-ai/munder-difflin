import { expect, test } from '@playwright/test';

test('redirects a delayed repair configuration from the office into the onboarding wizard', async ({ page }) => {
  await page.goto('/');
  await page.waitForFunction(() => {
    const main = document.querySelector('#main-content');
    return location.pathname === '/onboarding'
      || document.querySelector('.office-domain') !== null
      || main?.textContent?.includes('初期設定へ移動しています');
  });

  test.skip(
    await page.locator('.office-domain').isVisible(),
    'the configured fixture does not require onboarding repair',
  );

  await expect(page).toHaveURL(/\/onboarding$/);
  const wizard = page.locator('#config-onboarding');
  await expect(wizard).toBeVisible();
  await expect(wizard.getByRole('heading', { name: '最小チームとベースskill' })).toBeVisible();
  await wizard.getByRole('button', { name: '戻る' }).click();
  await expect(wizard.getByRole('heading', { name: 'リポジトリ' })).toBeVisible();
  await expect(wizard.getByRole('button', { name: '次へ' })).toBeVisible();
});
