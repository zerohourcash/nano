import { describe, expect, it } from "vitest";
import { buildXlsx, columnName } from "./xlsx";

/** Читает имена файлов из центрального каталога ZIP. */
function zipEntryNames(bytes: Uint8Array): string[] {
  const names: string[] = [];
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const decoder = new TextDecoder();
  for (let i = 0; i + 46 <= bytes.length; i++) {
    if (view.getUint32(i, true) !== 0x02014b50) continue; // запись каталога
    const nameLength = view.getUint16(i + 28, true);
    names.push(decoder.decode(bytes.subarray(i + 46, i + 46 + nameLength)));
  }
  return names;
}

function findPart(bytes: Uint8Array, part: string): string {
  const text = new TextDecoder().decode(bytes);
  const start = text.indexOf("<?xml", text.indexOf(part));
  expect(start).toBeGreaterThan(-1);
  return text.slice(start);
}

async function build(rows: (string | number | null)[][], sheet?: string) {
  const blob = buildXlsx(rows, sheet);
  return new Uint8Array(await blob.arrayBuffer());
}

describe("columnName", () => {
  it("нумерует столбцы как Excel", () => {
    expect(columnName(0)).toBe("A");
    expect(columnName(25)).toBe("Z");
    expect(columnName(26)).toBe("AA");
    expect(columnName(27)).toBe("AB");
  });
});

describe("buildXlsx", () => {
  it("выдаёт ZIP со всеми обязательными частями книги", async () => {
    const bytes = await build([["Наименование", "Кол-во"]]);

    // Сигнатура локального заголовка ZIP.
    expect(Array.from(bytes.slice(0, 4))).toEqual([0x50, 0x4b, 0x03, 0x04]);

    const names = zipEntryNames(bytes);
    expect(names).toEqual(
      expect.arrayContaining([
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/worksheets/sheet1.xml",
      ]),
    );
  });

  it("числа кладёт числами, а не текстом", async () => {
    const bytes = await build([["Перфоратор", 3]]);
    const sheet = findPart(bytes, "xl/worksheets/sheet1.xml");
    // Числовая ячейка — без t="inlineStr".
    expect(sheet).toContain("<c r=\"B1\"><v>3</v></c>");
    expect(sheet).toContain("t=\"inlineStr\"");
  });

  it("экранирует спецсимволы, не ломая XML", async () => {
    const bytes = await build([['Болт <М8> & "оцинк."']]);
    const sheet = findPart(bytes, "xl/worksheets/sheet1.xml");
    expect(sheet).toContain("Болт &lt;М8&gt; &amp; &quot;оцинк.&quot;");
    expect(sheet).not.toContain("<М8>");
  });

  it("пропускает пустые ячейки, сохраняя адреса остальных", async () => {
    const bytes = await build([["A", null, "C"]]);
    const sheet = findPart(bytes, "xl/worksheets/sheet1.xml");
    expect(sheet).toContain('r="A1"');
    expect(sheet).not.toContain('r="B1"');
    expect(sheet).toContain('r="C1"');
  });

  it("обрезает слишком длинное имя листа до предела Excel", async () => {
    const bytes = await build([["x"]], "О".repeat(50));
    const workbook = findPart(bytes, "xl/workbook.xml");
    const match = workbook.match(/name="([^"]*)"/);
    expect(match?.[1]?.length).toBe(31);
  });
});
