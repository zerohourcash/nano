import { expect, test, type Page } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';

async function trpc<T>(
  page: Page,
  procedure: string,
  input: unknown,
  mutation = true
): Promise<T> {
  return page.evaluate(
    async ({ procedure, input, mutation }) => {
      const envelope = JSON.stringify({ 0: { json: input } });
      const signedHeaders: Record<string, string> = {};
      if (mutation) {
        type Identity = { deviceId: string; privateKey: CryptoKey; publicKey: Uint8Array };
        const state = window as typeof window & { __everydayE2EIdentity?: Identity };
        if (!state.__everydayE2EIdentity) {
          const pair = (await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify'])) as CryptoKeyPair;
          state.__everydayE2EIdentity = {
            deviceId: `e2e-browser-${crypto.randomUUID()}`,
            privateKey: pair.privateKey,
            publicKey: new Uint8Array(await crypto.subtle.exportKey('raw', pair.publicKey)),
          };
        }
        const identity = state.__everydayE2EIdentity;
        const encode = (bytes: Uint8Array) => {
          let binary = '';
          for (const byte of bytes) binary += String.fromCharCode(byte);
          return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
        };
        const registration = await fetch('/api/trpc/auth.registerDevice', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ json: {
            deviceId: identity.deviceId,
            name: 'Playwright browser',
            publicKey: encode(identity.publicKey),
          } }),
        });
        if (!registration.ok) throw new Error(`device registration: HTTP ${registration.status}`);
        const timestamp = Math.floor(Date.now() / 1000).toString();
        const nonce = crypto.randomUUID();
        const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(envelope)));
        const hash = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
        const message = ['everyday/device-request/v1', 'POST', `/api/trpc/${procedure}`, timestamp, nonce, hash].join('\n');
        const signature = new Uint8Array(await crypto.subtle.sign('Ed25519', identity.privateKey, new TextEncoder().encode(message)));
        Object.assign(signedHeaders, {
          'x-everyday-device': identity.deviceId,
          'x-everyday-timestamp': timestamp,
          'x-everyday-nonce': nonce,
          'x-everyday-signature': encode(signature),
        });
      }
      const suffix = mutation
        ? '?batch=1'
        : `?batch=1&input=${encodeURIComponent(envelope)}`;
      const response = await fetch(`/api/trpc/${procedure}${suffix}`, {
        method: mutation ? 'POST' : 'GET',
        headers: mutation ? { 'content-type': 'application/json', ...signedHeaders } : undefined,
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
  test.setTimeout(90_000);
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
  const organizationDivision = await trpc<{ id: number }>(
    page,
    'admin.organizationNodes.create',
    {
      workspaceId: workspaces[0].id,
      kind: 'division',
      name: 'Сервисное подразделение',
      tabLabel: 'Сервис E2E',
      displayOrder: 10,
    }
  );
  const organizationRoom = await trpc<{ id: number }>(
    page,
    'admin.organizationNodes.create',
    {
      workspaceId: workspaces[0].id,
      parentId: organizationDivision.id,
      kind: 'room',
      name: 'Кабинет 204',
    }
  );

  await page.goto('/create');
  await page
    .getByPlaceholder('Например: Перфоратор Bosch GBH 8-45 DV')
    .fill('Перфоратор E2E');
  await page.locator('select').first().selectOption(String(category.id));
  await page.locator('input[type="file"]').first().setInputFiles({
    name: 'tool.png',
    mimeType: 'image/png',
    buffer: Buffer.from(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
      'base64'
    ),
  });
  const createDocumentInput = page.locator('input[type="file"]').nth(2);
  await createDocumentInput.setInputFiles({
    name: 'empty.pdf',
    mimeType: 'application/pdf',
    buffer: Buffer.alloc(0),
  });
  await expect(page.getByText(/Не добавлены: empty\.pdf.*1 байт–20 МБ/)).toBeVisible();
  await createDocumentInput.setInputFiles({
    name: 'manual.pdf',
    mimeType: 'application/pdf',
    buffer: Buffer.from('E2E signed document'),
  });
  await page.getByRole('button', { name: 'Создать инструмент' }).click();
  await expect(page).toHaveURL(/\/tool\/\d+/, { timeout: 10_000 });
  const itemId = Number(page.url().match(/\/tool\/(\d+)/)?.[1]);
  expect(itemId).toBeGreaterThan(0);
  await trpc(page, 'items.update', {
    id: itemId,
    organizationNodeId: organizationRoom.id,
  });
  await page.goto('/');
  await expect(page.getByTestId('organization-catalog-tabs')).toBeVisible();
  await page.getByRole('button', { name: 'Сервис E2E', exact: true }).click();
  await expect(page.getByRole('heading', { name: /Сервис E2E \(1 ед\.\)/ })).toBeVisible();
  await expect(page.getByText('Перфоратор E2E', { exact: true })).toBeVisible();
  await page.goto('/admin');
  await page.getByRole('button', { name: 'Структура', exact: true }).first().click();
  await page.getByRole('button', { name: 'Изменить Сервисное подразделение' }).click();
  await page.getByLabel('Название вкладки').fill('Ремонт E2E');
  await page.getByLabel('Тип раздела').selectOption('__custom__');
  await page.getByLabel('Собственный тип раздела').fill('мастерская');
  await page.getByLabel('Ответственный за раздел').selectOption({ label: 'Елена Владелец' });
  await page.getByRole('button', { name: 'Сохранить', exact: true }).click();
  await expect(page.getByText('Изменения раздела подписаны и сохранены')).toBeVisible();
  await expect(page.getByText(/мастерская.*ответственный: Елена Владелец/)).toBeVisible();
  await page.getByRole('button', { name: 'Пользователи', exact: true }).first().click();
  await page.getByRole('button', { name: 'Действия' }).first().click();
  await page.getByRole('menuitem', { name: 'Права доступа' }).click();
  await page.getByLabel('Должность в организации').fill('Главный инженер E2E');
  await page.getByLabel('Название роли').fill('Владелец объекта E2E');
  await page.getByLabel('Табельный номер').fill('OWNER-E2E-01');
  await page.getByLabel('Максимальный срок выдачи').fill('24');
  await page.getByRole('button', { name: 'Сохранить права' }).click();
  await expect(page.getByText('Права доступа обновлены')).toBeVisible();
  await expect(page.getByText('Главный инженер E2E', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Действия' }).first().click();
  await page.getByRole('menuitem', { name: 'Права доступа' }).click();
  await expect(page.getByLabel('Максимальный срок выдачи')).toHaveValue('24');
  await page.getByRole('button', { name: 'Отмена', exact: true }).click();
  await page.goto('/');
  await expect(page.getByRole('button', { name: 'Ремонт E2E', exact: true })).toBeVisible();
  await page.goto(`/tool/${itemId}`);
  await page
    .getByRole('button', { name: 'Взять', exact: true })
    .first()
    .click();
  await expect(page.getByText('Взять: Перфоратор E2E')).toBeVisible();
  const itemQr = await trpc<{ label: string }>(page, 'items.qrLabel', { itemId }, false);
  await page.getByPlaceholder('Или вставьте ссылку / токен').fill(itemQr.label);
  await page.getByRole('button', { name: 'Далее' }).click();
  await expect(page.getByText('QR получен и будет проверен сервером')).toBeVisible();
  await page.getByRole('checkbox').last().check();
  await page.getByRole('button', { name: 'Взять', exact: true }).last().click();
  await expect(page.getByText('Инструмент теперь у вас')).toBeVisible({
    timeout: 10_000,
  });
  await page.getByRole('button', { name: /Документы/ }).click();
  await page.getByLabel('Доступ к документу').selectOption('accounting');
  await page.getByTestId('tool-document-file').setInputFiles({
    name: 'invoice-e2e.pdf',
    mimeType: 'application/pdf',
    buffer: Buffer.from('E2E accounting invoice'),
  });
  await expect(page.getByText('Документ сохранён в CAS и подписан')).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByText('invoice-e2e.pdf', { exact: true })).toBeVisible();

  const itemAfterTake = await trpc<{
    documents: Array<{ name: string; url: string; mime?: string; accessLevel?: string }>;
    history: Array<{
      type: string;
      eventVersion?: number;
      requestDeviceId?: string;
      requestNonce?: string;
      requestHash?: string;
      qrProofs?: Array<{ itemId: number; version: number; sha256: string }>;
    }>;
  }>(page, 'items.byId', { id: itemId }, false);
  const event = itemAfterTake.history.find(
    entry => entry.type === 'transfer_receive'
  );
  expect(event).toMatchObject({ eventVersion: 3 });
  expect(event?.requestDeviceId).toBeTruthy();
  expect(event?.requestNonce).toBeTruthy();
  expect(event?.requestHash).toMatch(/^[a-f0-9]{64}$/);
  expect(event?.qrProofs).toEqual([{
    itemId,
    version: 2,
    sha256: createHash('sha256').update(itemQr.label).digest('hex'),
  }]);
  const photoEvent = itemAfterTake.history.find(entry => entry.type === 'photo_add');
  expect(photoEvent).toMatchObject({ eventVersion: 3 });
  expect(photoEvent?.requestDeviceId).toBeTruthy();
  expect(photoEvent?.requestHash).toMatch(/^[a-f0-9]{64}$/);
  expect(itemAfterTake.documents).toEqual(expect.arrayContaining([
    expect.objectContaining({
      name: 'manual.pdf',
      url: expect.stringMatching(/^data:application\/pdf;base64,/),
      mime: 'application/pdf',
    }),
    expect.objectContaining({
      name: 'invoice-e2e.pdf',
      url: expect.stringMatching(/^data:application\/pdf;base64,/),
      mime: 'application/pdf',
      accessLevel: 'accounting',
    }),
  ]));
  const documentEvent = itemAfterTake.history.find(entry => entry.type === 'document_add');
  expect(documentEvent).toMatchObject({ eventVersion: 3 });
  expect(documentEvent?.requestDeviceId).toBeTruthy();
  expect(documentEvent?.requestHash).toMatch(/^[a-f0-9]{64}$/);

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
    qrItem.history.find(entry => entry.type === 'item_state_create')?.requestDeviceId
  ).toBeTruthy();
  await page.getByRole('button', { name: 'Печать QR' }).click();
  await expect(page.locator('#item-qr-canvas')).toBeAttached({ timeout: 10_000 });
  const qrPngData = await page.evaluate(() => {
    const canvas = document.getElementById('item-qr-canvas') as HTMLCanvasElement | null;
    if (!canvas) throw new Error('export QR canvas is missing');
    return canvas.toDataURL('image/png');
  });
  const qrPng = Buffer.from(qrPngData.split(',')[1], 'base64');
  await page.getByRole('button', { name: 'Закрыть' }).click();
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
    page.getByText(/Шуруповёрт QR E2E · ВН-\d+ · на складе/)
  ).toBeVisible();
  await expect(page.getByText('Для выдачи нужна подписанная QR-бирка V2 — обратитесь к кладовщику')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Взять все (1)' })).toHaveCount(0);
  await page.locator('input[type="file"]').setInputFiles({
    name: 'signed-tool-qr.png',
    mimeType: 'image/png',
    buffer: qrPng,
  });
  await expect(page.getByRole('button', { name: 'Взять все (1)' })).toBeVisible();
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
      qrProofs?: Array<{ itemId: number; version: number; sha256: string }>;
    }>;
  }>(page, 'items.byId', { id: qrItem.id }, false);
  const qrEvent = qrItemAfterTake.history.find(
    entry => entry.type === 'transfer_receive'
  );
  expect(qrItemAfterTake.responsibleUserId).toBeTruthy();
  expect(qrEvent).toMatchObject({ eventVersion: 3 });
  expect(qrEvent?.requestDeviceId).toBeTruthy();
  expect(qrEvent?.requestHash).toMatch(/^[a-f0-9]{64}$/);
  expect(qrEvent?.qrProofs).toHaveLength(1);
  expect(qrEvent?.qrProofs?.[0]).toMatchObject({ itemId: qrItem.id, version: 2 });
  expect(qrEvent?.qrProofs?.[0].sha256).toMatch(/^[a-f0-9]{64}$/);

  const inventorySite = await trpc<{ id: number }>(page, 'admin.buildingSites.create', {
    workspaceId: workspaces[0].id,
    name: 'Корпус QR E2E',
  });
  await trpc(page, 'items.update', { id: qrItem.id, buildingSiteId: inventorySite.id });
  await page.goto('/inventory');
  await page.getByRole('button', { name: 'Новая инвентаризация' }).first().click();
  await page.getByRole('button', { name: 'Объект', exact: true }).click();
  await page.locator('select').filter({ has: page.locator(`option[value="${inventorySite.id}"]`) }).selectOption(String(inventorySite.id));
  await page.getByText('Блокировать передачи в области сверки до завершения').click();
  await page.getByRole('button', { name: 'Начать сверку' }).click();
  await expect(page.getByText('Инвентаризация ИНВ-001 начата')).toBeVisible();
  const inventories = await trpc<Array<{ id: number; number: string }>>(
    page, 'inventory.sessions', { workspaceId: workspaces[0].id }, false
  );
  const inventory = inventories.find(session => session.number === 'ИНВ-001');
  expect(inventory).toBeTruthy();
  await expect(
    trpc(page, 'transfers.returnItem', { itemId: qrItem.id })
  ).rejects.toThrow(/заблокированы инвентаризацией/);
  await page.getByRole('button', { name: 'Сканировать QR' }).first().click();
  const inventoryScanner = page.getByTestId('inventory-qr-scanner');
  await expect(inventoryScanner).toBeVisible();
  const inventoryCode = inventoryScanner.getByPlaceholder('Или вставьте ссылку / токен');
  await inventoryCode.fill('everyday:item:00000000-0000-4000-8000-000000000000');
  await inventoryScanner.getByRole('button', { name: 'Далее' }).click();
  await expect(page.getByText('QR не прошёл проверку или относится к другой организации')).toBeVisible();
  await inventoryScanner.locator('input[type="file"]').setInputFiles({
    name: 'real-tool-qr.png',
    mimeType: 'image/png',
    buffer: qrPng,
  });
  await expect(page.getByText('Отмечено: Шуруповёрт QR E2E')).toBeVisible({ timeout: 10_000 });
  await inventoryScanner.getByRole('button', { name: 'Закрыть сканер' }).click();
  const inventoryAfterScan = await trpc<{
    results: Array<{ itemId: number; checked: boolean; actualQty: number | null }>;
  }>(page, 'inventory.byId', { id: inventory!.id }, false);
  expect(inventoryAfterScan.results.find(result => result.itemId === qrItem.id)).toMatchObject({
    checked: true,
    actualQty: 1,
  });
  const inventoryHistory = await trpc<Array<{
    type: string;
    eventVersion: number;
    requestDeviceId?: string;
    requestHash?: string;
  }>>(page, 'history.all', { workspaceId: workspaces[0].id, limit: 500 }, false);
  expect(inventoryHistory.find(entry => entry.type === 'inventory_check')).toMatchObject({
    eventVersion: 3,
    requestDeviceId: expect.any(String),
    requestHash: expect.stringMatching(/^[a-f0-9]{64}$/),
  });
  await page.getByRole('button', { name: 'Завершить сверку' }).click();
  await expect(page.getByText('Итоги инвентаризации')).toBeVisible();
  const actDownload = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Скачать подписанный акт' }).last().click();
  const downloadedAct = await actDownload;
  expect(downloadedAct.suggestedFilename()).toBe('ИНВ-001-signed-act.json');
  const actPath = await downloadedAct.path();
  expect(actPath).toBeTruthy();
  const signedAct = JSON.parse(await readFile(actPath!, 'utf8')) as {
    format: string;
    act: unknown;
    canonical: string;
    hash: string;
    signature: string;
    publicKey: string;
    signatureDomain: string;
  };
  expect(signedAct.format).toBe('everyday-inventory-act');
  expect(await page.evaluate(async (document) => {
    const decode = (value: string) => Uint8Array.from(atob(value), char => char.charCodeAt(0));
    const bytes = decode(document.canonical);
    const canonicalAct = JSON.parse(new TextDecoder().decode(bytes));
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const hash = Array.from(digest, byte => byte.toString(16).padStart(2, '0')).join('');
    const key = await crypto.subtle.importKey('raw', decode(document.publicKey), 'Ed25519', false, ['verify']);
    const valid = await crypto.subtle.verify(
      'Ed25519', key, decode(document.signature),
      new TextEncoder().encode(`${document.signatureDomain}\n${document.hash}`)
    );
    return hash === document.hash && valid && JSON.stringify(canonicalAct) === JSON.stringify(document.act);
  }, signedAct)).toBe(true);
  await page.getByRole('button', { name: 'Все сессии' }).click();
  const actUpload = page.getByTestId('inventory-act-upload');
  await actUpload.setInputFiles({
    name: 'verified-act.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(signedAct)),
  });
  await expect(page.getByTestId('inventory-act-verification')).toContainText('Акт полностью подтверждён');
  const tamperedAct = structuredClone(signedAct) as typeof signedAct & { act: Record<string, unknown> };
  tamperedAct.act.number = 'ИНВ-ПОДМЕНА';
  await actUpload.setInputFiles({
    name: 'tampered-act.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(tamperedAct)),
  });
  await expect(page.getByTestId('inventory-act-verification')).toContainText('Подпись или содержимое акта повреждены');
  await trpc(page, 'transfers.returnItem', { itemId: qrItem.id });

  const bitRecipient = await trpc<{ id: number }>(page, 'admin.users.create', {
    workspaceId: workspaces[0].id,
    fullName: 'Иван Получатель Bit',
    phone: '79005550199',
    position: 'Мастер',
  });
  await trpc(page, 'admin.users.update', {
    workspaceId: workspaces[0].id,
    id: bitRecipient.id,
    status: 'active',
  });
  const saleItem = await trpc<{ id: number }>(page, 'items.create', {
    workspaceId: workspaces[0].id,
    internalId: 'BIT-SALE-E2E',
    title: 'Расходник для продажи Bit',
    responsibleUserId: bitRecipient.id,
  });
  const currentUser = await trpc<{ id: number }>(page, 'auth.me', null, false);
  const offeredItem = await trpc<{ id: number }>(page, 'items.create', {
    workspaceId: workspaces[0].id,
    internalId: 'BIT-OFFER-E2E',
    title: 'Материал для двухфазной продажи',
    quantitative: true,
    quantity: 10,
    unit: 'шт.',
  });
  const offeredQr = await trpc(page, 'items.qrLabel', { itemId: offeredItem.id }, false) as { label: string };
  await trpc(page, 'transfers.take', { itemId: offeredItem.id, qrLabel: offeredQr.label, quantity: 6 });
  await page.goto('/bit');
  await expect(page.getByRole('heading', { name: 'Кошелёк Bit' })).toBeVisible();
  await page.getByRole('button', { name: 'Эмиссия' }).click();
  await page.getByLabel('Получатель Bit').selectOption(String(currentUser.id));
  await page.getByLabel('Сумма Bit').fill('100');
  await page.getByLabel('Назначение платежа').fill('Стартовый фонд E2E');
  await page.getByRole('button', { name: 'Подписать транзакцию' }).click();
  await expect(page.getByTestId('bit-balance')).toHaveText('100 Bit');

  await page.getByRole('button', { name: 'Перевод' }).click();
  await page.getByLabel('Получатель Bit').selectOption(String(bitRecipient.id));
  await page.getByLabel('Сумма Bit').fill('25');
  await page.getByLabel('Назначение платежа').fill('Оплата смены');
  await page.getByRole('button', { name: 'Подписать транзакцию' }).click();
  await expect(page.getByTestId('bit-balance')).toHaveText('75 Bit');

  await page.getByRole('button', { name: 'Продажа' }).click();
  await page.getByLabel('Покупатель').selectOption(String(bitRecipient.id));
  await page.getByLabel('Товар или ТМЦ').selectOption(String(offeredItem.id));
  await page.getByLabel('Количество материала').fill('4');
  await page.getByLabel('Сумма Bit').fill('12');
  await page.getByLabel('Назначение платежа').fill('Подписанное предложение E2E');
  await page.getByRole('button', { name: 'Подписать транзакцию' }).click();
  await expect(page.getByTestId('bit-sale-offer')).toContainText('12 Bit');
  await expect(page.getByTestId('bit-balance')).toHaveText('75 Bit');
  await expect(page.getByTestId('bit-transaction')).toHaveCount(2);
  const pendingOffers = await trpc<Array<{
    id: number;
    status: string;
    bitAmount: number;
    quantity: number | null;
    bitTransactionGuid: string | null;
  }>>(page, 'bit.offers', { workspaceId: workspaces[0].id }, false);
  expect(pendingOffers).toEqual([
    expect.objectContaining({
      status: 'pending',
      bitAmount: 12,
      quantity: 4,
      bitTransactionGuid: null,
    }),
  ]);
  const materialBeforeAcceptance = await trpc<{
    holders: Array<{ userId: number; quantity: number }>;
  }>(page, 'items.byId', { id: offeredItem.id }, false);
  expect(materialBeforeAcceptance.holders.find(holder => holder.userId === currentUser.id)?.quantity).toBe(6);

  await page.getByRole('button', { name: 'Покупка' }).click();
  await page.getByLabel('Продавец').selectOption(String(bitRecipient.id));
  await expect(
    trpc(page, 'bit.sale', {
      itemId: saleItem.id,
      sellerUserId: currentUser.id,
      amount: 1,
      memo: 'Попытка подменить продавца',
    })
  ).rejects.toThrow(/не является ответственным/);
  await expect(page.getByTestId('bit-transaction')).toHaveCount(2);
  await page.getByLabel('Товар или ТМЦ').selectOption(String(saleItem.id));
  await page.getByLabel('Сумма Bit').fill('10');
  await page.getByLabel('Назначение платежа').fill('Покупка расходника');
  await page.getByRole('button', { name: 'Подписать транзакцию' }).click();
  await expect(page.getByTestId('bit-balance')).toHaveText('65 Bit');
  await expect(page.getByTestId('bit-transaction')).toHaveCount(3);
  const bitTransactions = await trpc<Array<{
    kind: string;
    status: string;
    senderUserId: number | null;
    recipientUserId: number | null;
  }>>(page, 'bit.transactions', { workspaceId: workspaces[0].id }, false);
  expect(bitTransactions).toHaveLength(3);
  expect(bitTransactions.every(transaction => transaction.status === 'posted')).toBe(true);
  expect(bitTransactions.find(transaction => transaction.kind === 'sale')).toMatchObject({
    senderUserId: currentUser.id,
    recipientUserId: bitRecipient.id,
  });
  const bitHistory = await trpc<Array<{
    type: string;
    eventVersion: number;
    requestDeviceId?: string;
    requestHash?: string;
  }>>(page, 'history.all', { workspaceId: workspaces[0].id, limit: 500 }, false);
  for (const type of ['bit_mint', 'bit_transfer', 'bit_sale']) {
    expect(bitHistory.find(event => event.type === type)).toMatchObject({
      eventVersion: 3,
      requestDeviceId: expect.any(String),
      requestHash: expect.stringMatching(/^[a-f0-9]{64}$/),
    });
  }
  const bitAudit = await trpc<{ accountingVerified: boolean; accountingError: string | null }>(
    page, 'sync.audit', null, false
  );
  expect(bitAudit).toMatchObject({ accountingVerified: true, accountingError: null });

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

  await page.goto('/chat');
  await page.getByRole('button', { name: 'Прикрепить файлы' }).click();
  await page.locator('input[type="file"]').setInputFiles({
    name: 'mesh-note.txt',
    mimeType: 'text/plain',
    buffer: Buffer.from('offline mesh attachment'),
  });
  await page.getByPlaceholder('Сообщение группе…').fill('Файл из локального чата');
  await page.getByRole('button', { name: 'Отправить' }).click();
  await expect(page.getByText('Файл из локального чата')).toBeVisible();
  await expect(page.getByRole('link', { name: 'mesh-note.txt' })).toBeVisible();

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

  const integrity = await trpc<{
    healthy: boolean;
    photoError?: string | null;
    documentIntentVerified?: boolean;
    knowledgeIntentVerified?: boolean;
    chatIntentVerified?: boolean;
  }>(
    page,
    'sync.audit',
    null,
    false
  );
  expect(integrity.healthy, JSON.stringify(integrity)).toBe(true);
  expect(integrity.documentIntentVerified).toBe(true);
  expect(integrity.knowledgeIntentVerified).toBe(true);
  expect(integrity.chatIntentVerified).toBe(true);

  await page.goto('/admin');
  await page.getByRole('button', { name: 'Сеть организаций' }).first().click();
  await expect(page.getByTestId('interorg-network')).toBeVisible();
  await expect(page.getByTestId('interorg-ble-send')).toBeVisible();
  await expect(page.getByTestId('interorg-file-export')).toBeVisible();
  const interorgImport = page.getByTestId('interorg-file-import');
  await interorgImport.setInputFiles({
    name: 'too-many-interorg-envelopes.json',
    mimeType: 'application/vnd.everyday.interorg+json',
    buffer: Buffer.from(JSON.stringify({
      format: 'everyday-interorg-gossip',
      version: 1,
      envelopes: Array.from({ length: 33 }, () => ({})),
    })),
  });
  await expect(page.getByText('В пакете больше 32 конвертов')).toBeVisible();
  await interorgImport.setInputFiles({
    name: 'empty-interorg-gossip.json',
    mimeType: 'application/vnd.everyday.interorg+json',
    buffer: Buffer.from(JSON.stringify({
      format: 'everyday-interorg-gossip',
      version: 1,
      envelopes: [],
    })),
  });
  await expect(page.getByText(/Пакет проверен: новых 0, доставлено 0, повторов 0/)).toBeVisible();
  await page.getByRole('button', { name: 'Создать адрес' }).click();
  await expect(page.getByTestId('organization-card')).toContainText(
    'everyday:org:',
    { timeout: 10_000 }
  );
  const organizationIdentity = await trpc<{
    destination: string;
    publicKey: string;
    signingKey: string;
  }>(page, 'interorg.identity', { workspaceId: workspaces[0].id }, false);
  expect(organizationIdentity.destination).toMatch(/^[a-f0-9]{64}$/);
  expect(organizationIdentity.publicKey.length).toBeGreaterThan(40);
  expect(organizationIdentity.signingKey.length).toBeGreaterThan(40);
  const interorgHistory = await trpc<Array<{
    type: string;
    eventVersion: number;
    requestDeviceId?: string;
    requestHash?: string;
  }>>(page, 'history.all', { workspaceId: workspaces[0].id, limit: 500 }, false);
  expect(interorgHistory.find(entry => entry.type === 'interorg_identity_create')).toMatchObject({
    eventVersion: 3,
    requestDeviceId: expect.any(String),
    requestHash: expect.stringMatching(/^[a-f0-9]{64}$/),
  });
  const remoteKeys = await page.evaluate(async () => {
    const signing = (await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify'])) as CryptoKeyPair;
    const encode = (bytes: Uint8Array) => {
      let binary = '';
      for (const byte of bytes) binary += String.fromCharCode(byte);
      return btoa(binary);
    };
    return {
      encryptionKey: encode(crypto.getRandomValues(new Uint8Array(32))),
      signingKey: encode(new Uint8Array(await crypto.subtle.exportKey('raw', signing.publicKey))),
    };
  });
  const e2eContact = await trpc<{ guid: string }>(page, 'interorg.trustContact', {
    workspaceId: workspaces[0].id,
    name: 'E2E Контрагент',
    remoteWorkspaceGuid: crypto.randomUUID(),
    ...remoteKeys,
  });
  await page.reload();
  await page.getByRole('button', { name: 'Сеть организаций' }).first().click();
  await expect(page.getByTestId('interorg-contact')).toContainText('E2E Контрагент');
  const interorgTransactionId = crypto.randomUUID();
  await trpc(page, 'interorg.send', {
    workspaceId: workspaces[0].id,
    contactGuid: e2eContact.guid,
    transactionId: interorgTransactionId,
    kind: 'invoice.offer',
    body: { text: 'Проверка автономной очереди' },
  });
  await page.reload();
  await page.getByRole('button', { name: 'Сеть организаций' }).first().click();
  const outgoingInterorg = page.getByTestId('interorg-outbox-item').filter({ hasText: interorgTransactionId });
  await expect(outgoingInterorg).toContainText('Ожидает квитанцию');
  await page.getByLabel('Контрагент').selectOption(e2eContact.guid);
  await page.getByPlaceholder('Текст сообщения или условия сделки').fill('Закрытый файл E2E');
  await page.getByTestId('interorg-file-input').setInputFiles({
    name: 'shift-e2e.txt',
    mimeType: 'text/plain',
    buffer: Buffer.from('encrypted interorg file from Playwright'),
  });
  await page.getByRole('button', { name: 'Подписать и отправить' }).click();
  await expect(page.getByTestId('interorg-outbox-item').filter({ hasText: 'message.file' })).toContainText('Ожидает квитанцию');
  await page.getByRole('button', { name: 'Отозвать ключи' }).click();
  await expect(page.getByTestId('interorg-contact')).toContainText('отозван');
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
