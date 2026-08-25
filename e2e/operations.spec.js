import { test, expect } from '@playwright/test';

const token='playwright-local-token-32-characters';
test.beforeEach(async({page})=>{
  await page.addInitScript(value=>localStorage.setItem('bitCommunityToken',value),token);
});

test('полный горячий сценарий склада и диагностики',async({page},testInfo)=>{
  const suffix=`${testInfo.project.name}-${Date.now()}`;
  const person=`Анна ${suffix}`;
  const serial=`DR-${suffix}`;
  const consoleErrors=[];
  page.on('console',m=>{if(m.type()==='error')consoleErrors.push(m.text())});

  await page.goto('/');
  await expect(page.getByRole('heading',{name:'Обзор организации'})).toBeVisible();
  await expect.poll(async()=>page.evaluate(async()=>((await fetch('/api/state',{headers:{authorization:`Bearer ${localStorage.getItem('bitCommunityToken')}`}})).status))).toBe(200);

  await page.getByRole('button',{name:/Участники/}).click();
  await page.getByRole('button',{name:'Добавить участника'}).click();
  await page.getByLabel('Имя').fill(person);
  await page.getByRole('button',{name:'Авторизовать'}).click();
  await expect(page.getByText(person)).toBeVisible();

  await page.getByRole('button',{name:/Оборудование/}).click();
  await page.getByRole('button',{name:'Добавить оборудование'}).click();
  await page.getByLabel('Название').fill('Аккумуляторная дрель');
  await page.getByLabel('Серийный номер').fill(serial);
  await page.getByLabel('Место хранения').fill('Склад А · стеллаж 2');
  await page.getByRole('button',{name:'Добавить',exact:true}).click();
  const card=page.locator('.asset-card').filter({hasText:serial});
  await expect(card).toContainText('Свободно');

  await card.getByRole('button',{name:'Взять'}).click();
  await page.getByLabel('Получатель').selectOption({label:person});
  await page.getByRole('button',{name:'Подтвердить и подписать'}).click();
  await expect(card).toContainText(`У ${person}`);

  await card.getByRole('button',{name:'Вернуть'}).click();
  await page.getByLabel('Место возврата').fill('Склад Б');
  await page.getByRole('button',{name:'Подтвердить и подписать'}).click();
  await expect(card).toContainText('Свободно');

  await page.getByRole('button',{name:/История/}).click();
  await expect(page.getByText('Оборудование выдано').first()).toBeVisible();
  await expect(page.getByText('Оборудование возвращено').first()).toBeVisible();
  expect(consoleErrors).toEqual([]);

  await page.evaluate(async value=>fetch('/api/action',{method:'POST',headers:{authorization:`Bearer ${value}`,'content-type':'application/json'},body:'{"type":"ASSET_CREATE","payload":{}}'}),token);
  await page.getByRole('button',{name:/Диагностика/}).click();
  await expect(page.getByText('Некорректное оборудование').first()).toBeVisible();
  await expect(page.locator('#errorList')).not.toContainText(token);
  await page.screenshot({path:`test-results/${testInfo.project.name}-hot-flow.png`,fullPage:true});
});

test('пустой экран остаётся понятным и адаптивным',async({page},testInfo)=>{
  await page.goto('/#diagnostics');
  await expect(page.getByRole('heading',{name:'Диагностика ноды'})).toBeVisible();
  await expect(page.getByText(/Ошибок не зарегистрировано|Некорректное оборудование/).first()).toBeVisible();
  const body=await page.locator('body').evaluate(el=>({scroll:el.scrollWidth,client:el.clientWidth}));
  expect(body.scroll).toBeLessThanOrEqual(body.client+1);
  await page.screenshot({path:`test-results/${testInfo.project.name}-diagnostics.png`,fullPage:true});
});
