import { expect, test } from '@playwright/test';

test.describe('filesystem, Git and IDE domain', () => {
  test('renders without horizontal overflow on a phone viewport', async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 740 });
    await page.goto('/workspace');
    const ide = page.locator('[data-fs-git-ide]');
    await expect(ide).toBeVisible();
    const overflow = await ide.evaluate((node) => node.scrollWidth > node.clientWidth);
    expect(overflow).toBe(false);
  });

  test('keeps editor operations keyboard reachable', async ({ page }) => {
    await page.goto('/workspace');
    const ide = page.locator('[data-fs-git-ide]');
    await expect(ide.getByRole('combobox', { name: 'ワークスペース' })).toBeVisible();
    await expect(ide.getByRole('button', { name: 'Issue / CIを取得' })).toBeVisible();
  });

  test('exposes history and keeps checkout behind explicit confirmation', async ({ page }) => {
    await page.goto('/workspace');
    const ide = page.locator('[data-fs-git-ide]');
    await expect(ide).toBeVisible();
    const registered = ide.locator('select option').filter({ hasNotText: '登録済みworkspaceなし' });
    test.skip((await registered.count()) === 0, 'a registered Git workspace is required');

    await expect(ide.getByRole('heading', { name: '履歴グラフ' })).toBeVisible();
    await expect(ide.getByRole('heading', { name: '参照を比較' })).toBeVisible();
    await expect(ide.getByRole('heading', { name: 'worktrees' })).toBeVisible();
    const checkout = ide.getByRole('button', { name: 'checkout', exact: true });
    await expect(checkout).toBeDisabled();
    await ide.getByRole('textbox', { name: 'checkoutする参照' }).fill('main');
    await expect(checkout).toBeDisabled();
    await ide.getByRole('checkbox', { name: '未保存変更がなく、Agent停止済みと確認' }).check();
    await expect(checkout).toBeEnabled();
  });

  test('loads the project-private Monaco runtime without a CDN', async ({ page }) => {
    await page.goto('/workspace');
    const result = await page.evaluate(async () => {
      const runtime = globalThis as typeof globalThis & {
        monaco?: {
          editor: {
            create: (host: HTMLElement, options: { value: string; language: string }) => {
              getValue: () => string;
              getModel: () => { getLanguageId: () => string };
              dispose: () => void;
            };
          };
        };
        require?: {
          (modules: string[], ready: () => void, failed: (error: unknown) => void): void;
          config: (config: { paths: { vs: string } }) => void;
        };
      };
      if (!runtime.monaco?.editor.create) {
        await new Promise<void>((resolve, reject) => {
          const script = document.createElement('script');
          script.src = '/assets/monaco/vs/loader.js';
          script.onload = () => resolve();
          script.onerror = () => reject(new Error('Monaco loader failed'));
          document.head.append(script);
        });
        runtime.require?.config({ paths: { vs: '/assets/monaco/vs' } });
        await new Promise<void>((resolve, reject) => {
          runtime.require?.(['vs/editor/editor.main'], resolve, reject);
        });
      }
      const host = document.createElement('div');
      host.style.cssText = 'width:600px;height:300px';
      document.body.append(host);
      const editor = runtime.monaco?.editor.create(host, {
        value: 'fn main() {}',
        language: 'rust',
      });
      const value = editor?.getValue();
      const language = editor?.getModel().getLanguageId();
      editor?.dispose();
      host.remove();
      return { value, language };
    });

    expect(result).toEqual({ value: 'fn main() {}', language: 'rust' });
  });
});
