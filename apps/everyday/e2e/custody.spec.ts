import { expect, test, type Page } from '@playwright/test';

async function trpc<T>(
  page: Page,
  procedure: string,
  input: unknown,
  mutation = true
): Promise<T> {
  return page.evaluate(
    async ({ procedure, input, mutation }) => {
      const envelope = JSON.stringify({ 0: { json: input } });
      const suffix = mutation
        ? '?batch=1'
        : `?batch=1&input=${encodeURIComponent(envelope)}`;
      const response = await fetch(`/api/trpc/${procedure}${suffix}`, {
        method: mutation ? 'POST' : 'GET',
        headers: mutation ? { 'content-type': 'application/json' } : undefined,
        body: mutation ? envelope : undefined,
      });
      if (!response.ok)
        throw new Error(
          `${procedure}: HTTP ${response.status} ${await response.text()}`
        );
      const payload = (await response.json()) as Array<{
        result?: { data?: { json?: unknown } };
        error?: { json?: { message?: string } };
      }>;
      if (payload[0]?.error)
        throw new Error(
          payload[0].error.json?.message ?? `${procedure} failed`
        );
      return payload[0]?.result?.data?.json as T;
    },
    { procedure, input, mutation }
  );
}

test('browser signs a real custody transaction and ledger retains its proof', async ({
  page,
}) => {
  await page.goto('/');
  await expect(
    page.getByRole('heading', { name: 'Новая организация' })
  ).toBeVisible();

  await page.getByPlaceholder('Алексей Кузнецов').fill('Елена Владелец');
  await page.getByPlaceholder('+7 (921) 555-01-42').fill('79001112233');
  await page.getByPlaceholder('Придумайте пароль').fill('StrongPassword123!');
  await page.getByPlaceholder('ООО «СтройМонтаж»').fill('E2E Объект');
  await page.getByText('Соглашаюсь с').click();
  await page.getByRole('button', { name: 'Создать организацию' }).click();
  await expect(page).toHaveURL(/\/$/, { timeout: 10_000 });

  const workspaces = await trpc<Array<{ id: number }>>(
    page,
    'meta.workspaces',
    null,
    false
  );
  const category = await trpc<{ id: number }>(
    page,
    'admin.dictionaries.create',
    {
      workspaceId: workspaces[0].id,
      kind: 'categories',
      name: 'Электроинструмент',
    }
  );

  await page.goto('/create');
  await page
    .getByPlaceholder('Например: Перфоратор Bosch GBH 8-45 DV')
    .fill('Перфоратор E2E');
  await page.locator('select').first().selectOption(String(category.id));
  await page.getByRole('button', { name: 'Создать инструмент' }).click();
  await expect(page).toHaveURL(/\/tool\/\d+/, { timeout: 10_000 });
  const itemId = Number(page.url().match(/\/tool\/(\d+)/)?.[1]);
  expect(itemId).toBeGreaterThan(0);
  await page
    .getByRole('button', { name: 'Взять', exact: true })
    .first()
    .click();
  await expect(page.getByText('Взять: Перфоратор E2E')).toBeVisible();
  await page.getByRole('checkbox').last().check();
  await page.getByRole('button', { name: 'Взять', exact: true }).last().click();
  await expect(page.getByText('Инструмент теперь у вас')).toBeVisible({
    timeout: 10_000,
  });

  const itemAfterTake = await trpc<{
    history: Array<{
      type: string;
      eventVersion?: number;
      requestDeviceId?: string;
      requestNonce?: string;
      requestHash?: string;
    }>;
  }>(page, 'items.byId', { id: itemId }, false);
  const event = itemAfterTake.history.find(
    entry => entry.type === 'transfer_receive'
  );
  expect(event).toMatchObject({ eventVersion: 2 });
  expect(event?.requestDeviceId).toBeTruthy();
  expect(event?.requestNonce).toBeTruthy();
  expect(event?.requestHash).toMatch(/^[a-f0-9]{64}$/);

  await page.goto('/create');
  await page
    .getByPlaceholder('Например: Перфоратор Bosch GBH 8-45 DV')
    .fill('Шуруповёрт QR E2E');
  await page.locator('select').first().selectOption(String(category.id));
  await page.getByRole('button', { name: 'Создать инструмент' }).click();
  await expect(page).toHaveURL(/\/tool\/\d+/, { timeout: 10_000 });
  const qrItemId = Number(page.url().match(/\/tool\/(\d+)/)?.[1]);
  const qrItem = await trpc<{
    id: number;
    internalId: string;
    guid: string;
    history: Array<{ type: string; requestDeviceId?: string }>;
  }>(page, 'items.byId', { id: qrItemId }, false);
  expect(
    qrItem.history.find(entry => entry.type === 'create')?.requestDeviceId
  ).toBeTruthy();
  await page.goto('/scan');
  const manualCode = page.getByPlaceholder('Или вставьте ссылку / токен');
  await manualCode.fill('everyday:item:00000000-0000-4000-8000-000000000000');
  await page.getByRole('button', { name: 'Далее' }).click();
  await expect(
    page.getByText('Инструмент не найден. Проверьте номер.')
  ).toBeVisible();
  await manualCode.fill(`everyday:item:${qrItem.guid}`);
  await page.getByRole('button', { name: 'Далее' }).click();
  await expect(
    page.getByText('Шуруповёрт QR E2E', { exact: true }).first()
  ).toBeVisible();
  page.once('dialog', dialog => dialog.accept(''));
  await page.getByRole('button', { name: 'Взять все (1)' }).click();
  await expect(page.getByText('Взято 1 шт.')).toBeVisible({ timeout: 10_000 });
  const qrItemAfterTake = await trpc<{
    responsibleUserId: number | null;
    history: Array<{
      type: string;
      eventVersion?: number;
      requestDeviceId?: string;
      requestHash?: string;
    }>;
  }>(page, 'items.byId', { id: qrItem.id }, false);
  const qrEvent = qrItemAfterTake.history.find(
    entry => entry.type === 'transfer_receive'
  );
  expect(qrItemAfterTake.responsibleUserId).toBeTruthy();
  expect(qrEvent).toMatchObject({ eventVersion: 2 });
  expect(qrEvent?.requestDeviceId).toBeTruthy();
  expect(qrEvent?.requestHash).toMatch(/^[a-f0-9]{64}$/);

  await page.goto('/knowledge');
  await expect(
    page.getByRole('heading', { name: 'База знаний' })
  ).toBeVisible();
  await page.getByRole('button', { name: 'Новая страница' }).click();
  await page.getByLabel('Название').fill('Безопасность E2E');
  await expect(page.getByLabel('Адрес страницы')).toHaveValue(
    'безопасность-e2e'
  );
  await page
    .getByLabel('Текст')
    .fill('# Проверка\n\nРаботает локально и подписывается устройством.');
  await page.locator('input[type="file"]').setInputFiles({
    name: 'checklist.txt',
    mimeType: 'text/plain',
    buffer: Buffer.from('offline checklist'),
  });
  await page.getByRole('button', { name: 'Подписать ревизию' }).click();
  await expect(page.getByTestId('knowledge-viewer')).toContainText(
    'Работает локально'
  );
  await expect(page.getByRole('link', { name: /checklist.txt/ })).toBeVisible();
  const knowledge = await trpc<{
    current: { revisionHash: string; attachments: Array<{ url: string }> };
  }>(
    page,
    'knowledge.bySlug',
    { workspaceId: workspaces[0].id, slug: 'безопасность-e2e' },
    false
  );
  expect(knowledge.current.revisionHash).toMatch(/^[a-f0-9]{64}$/);
  expect(knowledge.current.attachments[0]?.url).toMatch(
    /^data:text\/plain;base64,/
  );

  const transportBundle = await trpc<{
    format: string;
    version: number;
    cipher: string;
    kdf: string;
    nonce: string;
    ciphertext: string;
  }>(page, 'sync.exportBundle', null, false);
  expect(transportBundle.format).toBe('everyday-sync-bundle');
  expect(transportBundle.version).toBe(2);
  expect(transportBundle.cipher).toBe('XChaCha20-Poly1305');
  expect(transportBundle.kdf).toBe('HKDF-SHA256');
  expect(transportBundle.ciphertext.length).toBeGreaterThan(100);
  expect(JSON.stringify(transportBundle)).not.toContain('Безопасность E2E');
  const forgedBundle = structuredClone(transportBundle);
  forgedBundle.ciphertext = `${forgedBundle.ciphertext.startsWith('A') ? 'B' : 'A'}${forgedBundle.ciphertext.slice(1)}`;

  await page.goto('/admin');
  await page.getByRole('button', { name: 'Пространства', exact: true }).click();
  await expect(
    page.getByTitle('Скопировать GUID для organization scope ноды').first()
  ).toBeVisible();
  await page.getByRole('button', { name: 'Офлайн-узлы' }).first().click();
  await expect(
    page.getByRole('heading', { name: 'Целостность локальной копии' })
  ).toBeVisible();
  await expect(
    page.getByText(
      'Локальная история и текущее состояние криптографически согласованы'
    )
  ).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText(/Проверено подписей:/)).toBeVisible();
  await expect(page.getByText('Страниц знаний:')).toContainText('1');
  await expect(page.getByText('Ревизий знаний:')).toContainText('1');
  await expect(page.getByText(/Snapshot:/)).toContainText(/[a-f0-9]{64}/);
  await expect(
    page.getByRole('heading', { name: 'Ключи mesh-нод' })
  ).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Этот узел' })).toBeVisible();
  await expect(page.getByText(/Scope организаций:/)).toContainText('вся база');
  await expect(page.getByTestId('content-node-mode')).toContainText(
    'Подписанная летопись, транзакции и текст синхронизируются всегда'
  );
  await expect(page.getByTestId('content-node-mode')).toContainText(
    'Полная нода'
  );
  await expect(page.getByTestId('content-node-mode')).toContainText(
    'В каталоге:'
  );
  await expect(
    page.getByRole('heading', { name: 'Обмен без прямого соединения' })
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: 'Передать пакет' })
  ).toBeVisible();
  const transportInput = page.locator(
    'label:has-text("Принять пакет") input[type="file"]'
  );
  const validImportResponse = page.waitForResponse(response =>
    response.url().includes('sync.importBundle')
  );
  await transportInput.setInputFiles({
    name: 'valid-everyday-sync.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(transportBundle)),
  });
  const validImportPayload = await (await validImportResponse).json();
  const validImportResult = Array.isArray(validImportPayload)
    ? validImportPayload[0]
    : validImportPayload;
  expect(validImportResult?.error).toBeUndefined();
  const forgedImportResponse = page.waitForResponse(response =>
    response.url().includes('sync.importBundle')
  );
  await transportInput.setInputFiles({
    name: 'forged-everyday-sync.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(forgedBundle)),
  });
  const forgedImportPayload = await (await forgedImportResponse).json();
  const forgedImportResult = Array.isArray(forgedImportPayload)
    ? forgedImportPayload[0]
    : forgedImportPayload;
  expect(forgedImportResult?.error?.json?.message).toMatch(
    /повреждён|mesh-токен/
  );
  const diagnostics = await trpc<{
    unresolved: number;
    events: Array<{
      component: string;
      code: string;
      count: number;
      resolvedAt: string | null;
    }>;
  }>(page, 'sync.diagnostics', null, false);
  expect(diagnostics.unresolved).toBeGreaterThan(0);
  expect(diagnostics.events).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        component: 'transport',
        code: 'bundle_rejected',
        resolvedAt: null,
      }),
    ])
  );
  await expect(page.getByTestId('node-diagnostics')).toContainText(
    'bundle_rejected',
    { timeout: 10_000 }
  );
});
