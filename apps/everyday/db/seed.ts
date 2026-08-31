import { getDb } from "../api/queries/connection";
import {
  workspaces,
  users,
  userWorkspaces,
  storages,
  buildingSites,
  categories,
  brands,
  statuses,
  items,
  itemPhotos,
  itemDocuments,
  itemComments,
  transfers,
  inventorySessions,
  inventoryResults,
  notifications,
  DEFAULT_ROLE_RIGHTS,
} from "./schema";
import type { RoleRights } from "./schema";
import { appendHistory } from "../api/queries/history";

const D = (s: string) => new Date(s);

const fullRights: RoleRights = {
  viewItems: true, createItems: true, editItems: true, deleteItems: true,
  transferItems: true, acceptTransfers: true, writeOff: true, replenish: true,
  inventory: true, viewHistory: true, viewReports: true, manageUsers: true,
  manageWorkspaces: true, manageStorages: true, manageSites: true, manageDictionaries: true,
};

export async function seedDatabase() {
  const db = getDb();
  console.log("Seeding database...");

  const existing = await db.query.workspaces.findFirst();
  if (existing) {
    console.log("Database already seeded (workspaces found). Skipping.");
    return;
  }

  // ─── Рабочие пространства ────────────────────────────────────────────────
  const [ws1] = await db
    .insert(workspaces)
    .values({
      name: "ООО «СтройМонтаж»",
      timezone: "Europe/Moscow",
      internalIdPrefix: "ВН-",
      comment: "Основное рабочее пространство",
    })
    .returning();
  const [ws2] = await db
    .insert(workspaces)
    .values({
      name: "ИП «РемСервис»",
      timezone: "Europe/Moscow",
      internalIdPrefix: "РС-",
      comment: "Второе пространство",
    })
    .returning();
  const WS1 = ws1.id;
  console.log("workspaces:", WS1, ws2.id);

  // ─── Пользователи ──────────────────────────────────────────────────────────
  const usersData = [
    { fullName: "Алексей Кузнецов", position: "Кладовщик", phone: "+7 921 555-01-42", avatarUrl: "/avatar-1.png", roleRights: fullRights },
    { fullName: "Марина Орлова", position: "Прораб", phone: "+7 921 555-02-17", avatarUrl: "/avatar-2.png" },
    { fullName: "Игорь Савельев", position: "Мастер", phone: "+7 921 555-03-88", avatarUrl: "/avatar-3.png" },
    { fullName: "Ольга Демидова", position: "Руководитель", phone: "+7 921 555-04-29", avatarUrl: "/avatar-4.png", roleRights: fullRights },
    { fullName: "Павел Ким", position: "Монтажник", phone: "+7 921 555-05-63", avatarUrl: "/avatar-5.png" },
    { fullName: "Елена Ветрова", position: "Бухгалтер", phone: "+7 921 555-06-91", avatarUrl: "/avatar-6.png" },
  ];
  const insertedUsers = await db
    .insert(users)
    .values(
      usersData.map((u) => ({
        fullName: u.fullName,
        position: u.position,
        phone: u.phone,
        avatarUrl: u.avatarUrl,
        status: "active" as const,
        roleRights: u.roleRights ?? DEFAULT_ROLE_RIGHTS,
      })),
    )
    .returning();
  const U = insertedUsers.map((r) => r.id); // U[0] = Алексей Кузнецов
  console.log("users:", U);

  await db.insert(userWorkspaces).values([
    ...U.map((userId) => ({ userId, workspaceId: WS1 })),
    { userId: U[0], workspaceId: ws2.id },
  ]);

  // ─── Склады ────────────────────────────────────────────────────────────────
  const [wh1] = await db
    .insert(storages)
    .values({ name: "Центральный склад", responsibleUserId: U[0], workspaceId: WS1, address: "СПб, Индустриальный пр. 44" })
    .returning();
  const [wh2] = await db
    .insert(storages)
    .values({ name: "Склад №2", responsibleUserId: U[0], workspaceId: WS1, address: "Пушкин" })
    .returning();
  const WH: Record<string, number> = { "wh-1": wh1.id, "wh-2": wh2.id };

  // ─── Объекты ───────────────────────────────────────────────────────────────
  const sitesData = [
    { key: "site-1", name: "ЖК «Северная звезда»", responsibleUserId: U[1] },
    { key: "site-2", name: "БЦ «Лиговский 87»", responsibleUserId: U[2] },
    { key: "site-3", name: "ТРЦ «Галерея»", responsibleUserId: U[1] },
  ];
  const SITE: Record<string, number> = {};
  for (const s of sitesData) {
    const [row] = await db
      .insert(buildingSites)
      .values({ name: s.name, responsibleUserId: s.responsibleUserId, workspaceId: WS1 })
      .returning();
    SITE[s.key] = row.id;
  }

  // ─── Справочники ───────────────────────────────────────────────────────────
  const catsData = [
    { key: "cat-1", name: "Электроинструмент" },
    { key: "cat-2", name: "Измерительный и контрольный инструмент" },
    { key: "cat-3", name: "Оргтехника и компьютеры" },
    { key: "cat-4", name: "Ручной инструмент" },
    { key: "cat-5", name: "Расходные материалы" },
  ];
  const CAT: Record<string, number> = {};
  for (const c of catsData) {
    const [row] = await db.insert(categories).values({ name: c.name, workspaceId: WS1 }).returning();
    CAT[c.key] = row.id;
  }

  const brandsData = [
    { key: "br-1", name: "Bosch" },
    { key: "br-2", name: "Makita" },
    { key: "br-3", name: "DeWalt" },
    { key: "br-4", name: "Karcher" },
    { key: "br-5", name: "Metabo" },
    { key: "br-6", name: "Зубр" },
  ];
  const BR: Record<string, number> = {};
  for (const b of brandsData) {
    const [row] = await db.insert(brands).values({ name: b.name, workspaceId: WS1 }).returning();
    BR[b.key] = row.id;
  }

  const statusesData = [
    { slug: "in-work", name: "В работе", color: "#2E9E5B", bg: "#C8FCD2" },
    { slug: "in-repair", name: "В ремонте", color: "#A87C0F", bg: "#FBFCC8" },
    { slug: "in-stock", name: "На складе", color: "#5E629B", bg: "#EDEDF7" },
    { slug: "written-off", name: "Списан", color: "#D64545", bg: "#FAD8D1" },
  ];
  const ST: Record<string, number> = {};
  for (const s of statusesData) {
    const [row] = await db
      .insert(statuses)
      .values({ name: s.name, slug: s.slug, color: s.color, bg: s.bg, workspaceId: WS1 })
      .returning();
    ST[s.slug] = row.id;
  }

  // ─── Инструменты (соответствуют src/lib/mock-data.ts) ─────────────────────
  type SeedTool = {
    key: string; vn: string; name: string; photo: string;
    cat: string; br: string; st: string; site: string | null; wh: string;
    user: number | null; isMaterial: boolean; qty?: number; unit?: string;
    hasQr: boolean; price: number; serial?: string; createdAt: string;
  };
  const toolsData: SeedTool[] = [
    { key: "t-0142", vn: "ВН-0142", name: "Перфоратор Bosch GBH 8-45 DV", photo: "/tool-bosch-gbh.png", cat: "cat-1", br: "br-1", st: "in-work", site: "site-1", wh: "wh-1", user: U[1], isMaterial: false, hasQr: true, price: 68400, serial: "GBH-8845-2210", createdAt: "2024-03-12" },
    { key: "t-0087", vn: "ВН-0087", name: "Шуруповёрт аккумуляторный Makita DF333", photo: "/tool-makita-df.png", cat: "cat-1", br: "br-2", st: "in-work", site: "site-1", wh: "wh-1", user: U[2], isMaterial: false, hasQr: true, price: 18900, serial: "MK-DF333-7741", createdAt: "2023-11-02" },
    { key: "t-0201", vn: "ВН-0201", name: "Мойка высокого давления Karcher K5", photo: "/tool-karcher-k5.png", cat: "cat-1", br: "br-4", st: "in-stock", site: null, wh: "wh-1", user: null, isMaterial: false, hasQr: true, price: 32500, serial: "KR-K5-0093", createdAt: "2024-06-21" },
    { key: "t-0115", vn: "ВН-0115", name: "Торцовочная пила DeWalt DWS780", photo: "/tool-dewalt-dws.png", cat: "cat-1", br: "br-3", st: "in-repair", site: null, wh: "wh-2", user: null, isMaterial: false, hasQr: false, price: 89700, serial: "DW-780-5124", createdAt: "2023-08-14" },
    { key: "t-0156", vn: "ВН-0156", name: "Ноутбук MSI Modern 15", photo: "/tool-msi-laptop.png", cat: "cat-3", br: "br-1", st: "in-work", site: "site-2", wh: "wh-1", user: U[3], isMaterial: false, hasQr: true, price: 74900, serial: "MSI-15-9921", createdAt: "2024-01-30" },
    { key: "t-0063", vn: "ВН-0063", name: "Лазерный уровень Зубр ЛУ-360", photo: "/tool-laser-level.png", cat: "cat-2", br: "br-6", st: "in-stock", site: null, wh: "wh-2", user: null, isMaterial: false, hasQr: true, price: 12700, serial: "ZB-360-3355", createdAt: "2023-05-19" },
    { key: "t-0178", vn: "ВН-0178", name: "Углошлифмашина Metabo W 650", photo: "/tool-metabo-grinder.png", cat: "cat-1", br: "br-5", st: "in-work", site: "site-3", wh: "wh-1", user: U[4], isMaterial: false, hasQr: false, price: 9800, serial: "MT-W650-1187", createdAt: "2024-04-08" },
    { key: "t-0231", vn: "ВН-0231", name: "Тряпки ветошь 30×30, упаковка 100 шт", photo: "/tool-rags.png", cat: "cat-5", br: "br-6", st: "in-stock", site: null, wh: "wh-1", user: null, isMaterial: true, qty: 96, unit: "шт", hasQr: false, price: 1450, createdAt: "2024-09-03" },
    { key: "t-0088", vn: "ВН-0088", name: "Шуруповёрт аккумуляторный Makita DF333", photo: "/tool-makita-df.png", cat: "cat-1", br: "br-2", st: "in-stock", site: null, wh: "wh-1", user: null, isMaterial: false, hasQr: true, price: 18900, serial: "MK-DF333-7742", createdAt: "2024-02-11" },
    { key: "t-0143", vn: "ВН-0143", name: "Перфоратор Bosch GBH 8-45 DV", photo: "/tool-bosch-gbh.png", cat: "cat-1", br: "br-1", st: "in-repair", site: null, wh: "wh-2", user: null, isMaterial: false, hasQr: true, price: 68400, serial: "GBH-8845-2211", createdAt: "2024-05-27" },
    { key: "t-0179", vn: "ВН-0179", name: "Углошлифмашина Metabo W 650", photo: "/tool-metabo-grinder.png", cat: "cat-1", br: "br-5", st: "in-work", site: "site-1", wh: "wh-1", user: U[1], isMaterial: false, hasQr: true, price: 9800, serial: "MT-W650-1188", createdAt: "2024-04-22" },
    { key: "t-0202", vn: "ВН-0202", name: "Мойка высокого давления Karcher K5", photo: "/tool-karcher-k5.png", cat: "cat-1", br: "br-4", st: "in-work", site: "site-3", wh: "wh-1", user: U[4], isMaterial: false, hasQr: true, price: 32500, serial: "KR-K5-0094", createdAt: "2024-07-15" },
    { key: "t-0064", vn: "ВН-0064", name: "Лазерный уровень Зубр ЛУ-360", photo: "/tool-laser-level.png", cat: "cat-2", br: "br-6", st: "in-work", site: "site-2", wh: "wh-1", user: U[2], isMaterial: false, hasQr: true, price: 12700, serial: "ZB-360-3356", createdAt: "2023-06-30" },
    { key: "t-0157", vn: "ВН-0157", name: "Ноутбук MSI Modern 15", photo: "/tool-msi-laptop.png", cat: "cat-3", br: "br-1", st: "in-stock", site: null, wh: "wh-1", user: null, isMaterial: false, hasQr: true, price: 74900, serial: "MSI-15-9922", createdAt: "2024-03-19" },
    { key: "t-0232", vn: "ВН-0232", name: "Тряпки ветошь 30×30, упаковка 100 шт", photo: "/tool-rags.png", cat: "cat-5", br: "br-6", st: "in-stock", site: null, wh: "wh-2", user: null, isMaterial: true, qty: 54, unit: "шт", hasQr: false, price: 1450, createdAt: "2024-09-10" },
    { key: "t-0116", vn: "ВН-0116", name: "Торцовочная пила DeWalt DWS780", photo: "/tool-dewalt-dws.png", cat: "cat-1", br: "br-3", st: "in-work", site: "site-1", wh: "wh-1", user: U[4], isMaterial: false, hasQr: true, price: 89700, serial: "DW-780-5125", createdAt: "2023-10-05" },
    { key: "t-0089", vn: "ВН-0089", name: "Шуруповёрт аккумуляторный Makita DF333", photo: "/tool-makita-df.png", cat: "cat-1", br: "br-2", st: "written-off", site: null, wh: "wh-1", user: null, isMaterial: false, hasQr: true, price: 18900, serial: "MK-DF333-7743", createdAt: "2022-12-01" },
    { key: "t-0144", vn: "ВН-0144", name: "Перфоратор Bosch GBH 8-45 DV", photo: "/tool-bosch-gbh.png", cat: "cat-1", br: "br-1", st: "in-work", site: "site-3", wh: "wh-1", user: U[2], isMaterial: false, hasQr: true, price: 68400, serial: "GBH-8845-2212", createdAt: "2024-08-02" },
    { key: "t-0180", vn: "ВН-0180", name: "Углошлифмашина Metabo W 650", photo: "/tool-metabo-grinder.png", cat: "cat-1", br: "br-5", st: "in-stock", site: null, wh: "wh-1", user: null, isMaterial: false, hasQr: false, price: 9800, serial: "MT-W650-1189", createdAt: "2024-06-11" },
    { key: "t-0203", vn: "ВН-0203", name: "Мойка высокого давления Karcher K5", photo: "/tool-karcher-k5.png", cat: "cat-1", br: "br-4", st: "in-stock", site: null, wh: "wh-2", user: null, isMaterial: false, hasQr: true, price: 32500, serial: "KR-K5-0095", createdAt: "2024-10-01" },
  ];

  const T: Record<string, number> = {};
  for (const t of toolsData) {
    const [row] = await db
      .insert(items)
      .values({
        internalId: t.vn,
        title: t.name,
        categoryId: CAT[t.cat],
        brandId: BR[t.br],
        statusId: ST[t.st],
        responsibleUserId: t.user,
        buildingSiteId: t.site ? SITE[t.site] : null,
        storageId: WH[t.wh],
        workspaceId: WS1,
        serialNumber: t.serial ?? null,
        cost: t.price,
        quantitative: t.isMaterial,
        quantity: t.qty ?? null,
        unit: t.unit ?? null,
        qrCode: t.hasQr ? t.vn : null,
        createdAt: D(t.createdAt),
      })
      .returning();
    T[t.key] = row.id;
    await db.insert(itemPhotos).values({ itemId: row.id, url: t.photo, isTitle: true });
  }
  console.log("items:", Object.keys(T).length);

  // Документы и комментарии для карточек
  await db.insert(itemDocuments).values([
    { itemId: T["t-0142"], name: "Инструкция Bosch GBH 8-45 DV.pdf", url: "/docs/bosch-gbh-manual.pdf" },
    { itemId: T["t-0142"], name: "Гарантийный талон.pdf", url: "/docs/bosch-gbh-warranty.pdf" },
    { itemId: T["t-0156"], name: "Акт приёма-передачи.pdf", url: "/docs/msi-act.pdf" },
  ]);
  await db.insert(itemComments).values([
    { itemId: T["t-0142"], userId: U[1], text: "После работ на корпусе 3 вернуть на центральный склад.", createdAt: D("2025-08-18T10:30:00") },
    { itemId: T["t-0142"], userId: U[0], text: "Проверил комплектность, бур в кейсе.", createdAt: D("2025-08-18T09:40:00") },
    { itemId: T["t-0115"], userId: U[2], text: "Диск затупился, нужен новый перед ремонтом.", createdAt: D("2025-08-06T12:10:00") },
  ]);

  // ─── Передачи ──────────────────────────────────────────────────────────────
  const transfersData = [
    { code: "ПП-0042", item: "t-0142", from: U[0], to: U[1], site: "site-1", wh: "wh-1", status: "pending" as const, createdAt: "2025-08-18T09:24:00", comment: "Для корпуса 3" },
    { code: "ПП-0043", item: "t-0178", from: U[0], to: U[4], site: "site-3", wh: "wh-1", status: "pending" as const, createdAt: "2025-08-19T12:05:00", comment: null },
    { code: "ПП-0044", item: "t-0088", from: U[0], to: U[2], site: "site-2", wh: "wh-1", status: "pending" as const, createdAt: "2025-08-20T08:00:00", comment: null },
    { code: "ПП-0039", item: "t-0063", from: U[2], to: U[0], site: null, wh: "wh-2", status: "pending" as const, createdAt: "2025-08-17T10:12:00", comment: null },
    { code: "ПП-0040", item: "t-0156", from: U[3], to: U[0], site: null, wh: "wh-1", status: "pending" as const, createdAt: "2025-08-18T09:03:00", comment: null },
    { code: "ПП-0035", item: "t-0201", from: U[0], to: U[1], site: "site-1", wh: "wh-1", status: "accepted" as const, createdAt: "2025-08-10T15:02:00", comment: null },
    { code: "ПП-0031", item: "t-0115", from: U[4], to: U[0], site: null, wh: "wh-2", status: "rejected" as const, createdAt: "2025-08-05T14:00:00", comment: "Повреждён диск" },
  ];
  await db.insert(transfers).values(
    transfersData.map((t) => ({
      code: t.code,
      itemId: T[t.item],
      fromUserId: t.from,
      toUserId: t.to,
      toStorageId: WH[t.wh],
      buildingSiteId: t.site ? SITE[t.site] : null,
      workspaceId: WS1,
      status: t.status,
      comment: t.comment,
      createdAt: D(t.createdAt),
      completedAt: t.status === "accepted" || t.status === "rejected" ? D("2025-08-21T10:00:00") : null,
    })),
  );
  console.log("transfers:", transfersData.length);

  // ─── История (append-only, hash-цепочка) ──────────────────────────────────
  const names = ["Алексей Кузнецов", "Марина Орлова", "Игорь Савельев", "Ольга Демидова", "Павел Ким", "Елена Ветрова"];
  const historyData: Array<{
    item: string; type: "move" | "transfer_send" | "transfer_receive" | "write_off" | "replenish" | "inventory" | "create" | "update";
    actor: number; date: string; comment: string;
    fromLabel?: string; toLabel?: string; quantityDelta?: number;
  }> = [
    { item: "t-0203", type: "create", actor: U[0], date: "2024-10-01T10:00:00", comment: "Инструмент добавлен в каталог", toLabel: "Мойка высокого давления Karcher K5" },
    { item: "t-0156", type: "update", actor: U[3], date: "2025-07-20T13:30:00", comment: "Обновлены документы и фото" },
    { item: "t-0089", type: "write_off", actor: U[3], date: "2025-07-29T16:55:00", comment: "Списание: физический износ", fromLabel: "Шуруповёрт аккумуляторный Makita DF333" },
    { item: "t-0231", type: "replenish", actor: U[0], date: "2025-08-04T08:15:00", comment: "Пополнение: +20 шт (закупка)", toLabel: "Тряпки ветошь 30×30", quantityDelta: 20 },
    { item: "t-0115", type: "update", actor: U[0], date: "2025-08-06T11:40:00", comment: "Статус изменён: В работе → В ремонте", fromLabel: "В работе", toLabel: "В ремонте" },
    { item: "t-0201", type: "transfer_receive", actor: U[1], date: "2025-08-10T15:02:00", comment: "Приём от Алексея Кузнецова (ПП-0035)", fromLabel: names[0], toLabel: names[1] },
    { item: "t-0142", type: "transfer_send", actor: U[0], date: "2025-08-18T09:24:00", comment: "Передача Марине Орловой (ПП-0042)", fromLabel: names[0], toLabel: names[1] },
    { item: "t-0178", type: "transfer_send", actor: U[0], date: "2025-08-19T12:05:00", comment: "Передача Павлу Киму (ПП-0043)", fromLabel: names[0], toLabel: names[4] },
  ];
  for (const h of historyData) {
    await appendHistory(db, {
      workspaceId: WS1,
      itemId: T[h.item],
      type: h.type,
      actorUserId: h.actor,
      fromLabel: h.fromLabel ?? null,
      toLabel: h.toLabel ?? null,
      quantityDelta: h.quantityDelta ?? null,
      comment: h.comment,
      createdAt: D(h.date),
    });
  }
  console.log("history:", historyData.length);

  // ─── Инвентаризация ────────────────────────────────────────────────────────
  const [inv1] = await db
    .insert(inventorySessions)
    .values({ number: "ИНВ-001", workspaceId: WS1, status: "completed", startedBy: U[0], createdAt: D("2025-07-15T09:00:00"), completedAt: D("2025-07-15T16:30:00") })
    .returning();
  const [inv2] = await db
    .insert(inventorySessions)
    .values({ number: "ИНВ-002", workspaceId: WS1, status: "in_progress", startedBy: U[0], createdAt: D("2025-08-20T09:00:00") })
    .returning();

  const inv1Items = ["t-0142", "t-0087", "t-0201", "t-0115", "t-0156", "t-0063", "t-0178", "t-0231"];
  await db.insert(inventoryResults).values(
    inv1Items.map((k) => ({
      sessionId: inv1.id,
      itemId: T[k],
      expectedQty: k === "t-0231" ? 96 : 1,
      actualQty: k === "t-0231" ? 96 : 1,
      checked: true,
    })),
  );
  const inv2Items = ["t-0142", "t-0087", "t-0201", "t-0231", "t-0232", "t-0179"];
  await db.insert(inventoryResults).values(
    inv2Items.map((k, i) => ({
      sessionId: inv2.id,
      itemId: T[k],
      expectedQty: k === "t-0231" ? 96 : k === "t-0232" ? 54 : 1,
      actualQty: i < 2 ? (k === "t-0231" ? 96 : 1) : null,
      checked: i < 2,
    })),
  );
  console.log("inventory sessions:", inv1.id, inv2.id);

  // ─── Уведомления ───────────────────────────────────────────────────────────
  const notifData = [
    { userId: U[0], itemId: T["t-0063"], type: "transfer", title: "Ожидает приёма", text: "Передача ПП-0039: Лазерный уровень Зубр от Игоря Савельева", date: "2025-08-17T10:12:00", read: false },
    { userId: U[0], itemId: T["t-0156"], type: "transfer", title: "Ожидает приёма", text: "Передача ПП-0040: Ноутбук MSI от Ольги Демидовой", date: "2025-08-18T09:03:00", read: false },
    { userId: U[0], itemId: T["t-0063"], type: "reminder", title: "Напоминание о поверке", text: "Лазерный уровень Зубр ЛУ-360 — поверка до 01.09.2025", date: "2025-08-16T08:00:00", read: false },
    { userId: U[0], itemId: null, type: "inventory", title: "Инвентаризация запланирована", text: "Центральный склад — сверка начнётся 25.08.2025", date: "2025-08-15T14:45:00", read: true },
    { userId: U[0], itemId: null, type: "system", title: "Синхронизация завершена", text: "Журнал операций сохранён на сервере", date: "2025-08-14T18:20:00", read: true },
    { userId: U[0], itemId: T["t-0201"], type: "transfer", title: "Передача принята", text: "Марина Орлова приняла Мойку Karcher K5 (ПП-0035)", date: "2025-08-10T15:05:00", read: true },
    { userId: U[0], itemId: T["t-0142"], type: "reminder", title: "Напоминание о ТО", text: "Перфоратор Bosch GBH 8-45 DV — плановое ТО 15.09.2025", date: "2025-08-13T08:00:00", read: true },
    { userId: U[0], itemId: T["t-0089"], type: "system", title: "Списание выполнено", text: "Шуруповёрт Makita DF333 (ВН-0089) списан: физический износ", date: "2025-07-29T17:00:00", read: true },
    { userId: U[1], itemId: T["t-0142"], type: "transfer", title: "Ожидает приёма", text: "Передача ПП-0042: Перфоратор Bosch от Алексея Кузнецова", date: "2025-08-18T09:25:00", read: false },
    { userId: U[4], itemId: T["t-0178"], type: "transfer", title: "Ожидает приёма", text: "Передача ПП-0043: УШМ Metabo от Алексея Кузнецова", date: "2025-08-19T12:06:00", read: false },
  ];
  await db.insert(notifications).values(
    notifData.map((n) => ({
      userId: n.userId,
      itemId: n.itemId,
      type: n.type,
      title: n.title,
      text: n.text,
      read: n.read,
      createdAt: D(n.date),
    })),
  );
  console.log("notifications:", notifData.length);

  console.log("Done.");
}

const invokedDirectly = process.argv[1]?.replace(/\\/g, "/").includes("/db/seed");
if (invokedDirectly) {
  seedDatabase()
    .then(() => process.exit(0))
    .catch((err) => {
      console.error(err);
      process.exit(1);
    });
}
