import { expect, test } from "@playwright/test";

async function focusEditor(page) {
  await expect(async () => {
    await page.locator("canvas").click({ position: { x: 760, y: 274 } });
    await expect(page.locator("input")).toBeFocused();
  }).toPass();
}

async function selectAll(page) {
  const mac = await page.evaluate(() => /^(Mac|iP)/.test(navigator.platform));
  await page.keyboard.press(`${mac ? "Meta" : "Control"}+a`);
}

async function clipboard(page, type, text = "") {
  return page.evaluate(({ type, text }) => {
    const data = new DataTransfer();
    if (type === "paste") data.setData("text/plain", text);
    document.activeElement.dispatchEvent(new ClipboardEvent(type, {
      clipboardData: data, bubbles: true, cancelable: true,
    }));
    return data.getData("text/plain");
  }, { type, text });
}

async function composition(page, type, data) {
  await page.locator("input").evaluate((element, { type, data }) => {
    element.dispatchEvent(new CompositionEvent(type, { data, bubbles: true }));
  }, { type, data });
}

async function selectedText(page) {
  await selectAll(page);
  return clipboard(page, "copy");
}

async function plotImage(page) {
  const box = await page.locator("canvas").boundingBox();
  return page.screenshot({ clip: { x: box.x + 28, y: box.y + 90, width: 630, height: 535 } });
}

const browserErrors = new WeakMap();

test.beforeEach(async ({ page }) => {
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  browserErrors.set(page, errors);
  await page.goto("/");
  await focusEditor(page);
});

test.afterEach(async ({ page }) => {
  expect(browserErrors.get(page)).toEqual([]);
});

test("typing, clipboard, debounce, invalid markup, and Escape", async ({ page }) => {
  expect(await selectedText(page)).toBe("*Radius* $sqrt(x^2+y^2)$");
  const original = await plotImage(page);
  await page.keyboard.type("*Browser* $sqrt(x^2+y^2)$");
  await page.waitForTimeout(500);
  const applied = await plotImage(page);
  expect(applied.equals(original)).toBe(false);
  expect(await selectedText(page)).toBe("*Browser* $sqrt(x^2+y^2)$");
  expect(await clipboard(page, "cut")).toBe("*Browser* $sqrt(x^2+y^2)$");
  expect(await selectedText(page)).toBe("");
  await clipboard(page, "paste", "*Browser* $sqrt(x^2+y^2)$");
  await page.waitForTimeout(500);
  expect(await selectedText(page)).toBe("*Browser* $sqrt(x^2+y^2)$");
  await page.keyboard.type("$sqrt(x");
  await page.waitForTimeout(500);
  expect((await plotImage(page)).equals(applied)).toBe(true);
  await page.keyboard.press("Enter");
  await expect(page.locator("input")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.locator("input")).not.toBeFocused();
  await focusEditor(page);
  expect(await selectedText(page)).toBe("*Browser* $sqrt(x^2+y^2)$");
});

test("composition commits once and focus can leave the canvas", async ({ page }) => {
  await selectAll(page);
  await page.keyboard.type("Caf");
  const input = page.locator("input");
  await composition(page, "compositionstart", "");
  await composition(page, "compositionupdate", "é");
  const acceptsImeEnter = await input.evaluate(element => element.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true }),
  ));
  expect(acceptsImeEnter).toBe(true);
  await composition(page, "compositionend", "é");
  await input.evaluate(element => element.dispatchEvent(new InputEvent("input", { data: "é", inputType: "insertCompositionText", bubbles: true })));
  expect(await selectedText(page)).toBe("Café");
  await page.keyboard.press("End");
  await composition(page, "compositionstart", "");
  await composition(page, "compositionupdate", "x");
  await page.locator("#slow-loads").focus();
  await expect(page.locator("#slow-loads")).toBeFocused();
  await page.waitForTimeout(100);
  await expect(page.locator("#slow-loads")).toBeFocused();
  await focusEditor(page);
  expect(await selectedText(page)).toBe("Café");
});

test("panning moves the plot and annotation dragging stays independent", async ({ page }) => {
  await page.keyboard.press("Escape");
  const box = await page.locator("canvas").boundingBox();
  const before = await plotImage(page);
  await page.mouse.move(box.x + 160, box.y + 210);
  await page.mouse.down();
  await page.mouse.move(box.x + 195, box.y + 230, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(100);
  expect((await plotImage(page)).equals(before)).toBe(false);
  // The initial annotation is at (313, 213); panning moved it by (35, 20).
  const panned = await plotImage(page);
  const axisClip = { x: box.x + 20, y: box.y + 585, width: 660, height: 45 };
  const pannedAxis = await page.screenshot({ clip: axisClip });
  await page.mouse.move(box.x + 370, box.y + 230);
  await page.mouse.down();
  await page.mouse.move(box.x + 340, box.y + 205, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(100);
  expect((await plotImage(page)).equals(panned)).toBe(false);
  expect((await page.screenshot({ clip: axisClip })).equals(pannedAxis)).toBe(true);
});

test("late sample A cannot replace B and the new editor accepts input", async ({ page }) => {
  await page.goto("/?slow-loads");
  await focusEditor(page);
  const sampleA = await plotImage(page);
  await page.locator("canvas").click({ position: { x: 790, y: 51 } });
  await page.locator("canvas").click({ position: { x: 910, y: 51 } });
  await page.waitForTimeout(500);
  const sampleB = await plotImage(page);
  expect(sampleB.equals(sampleA)).toBe(false);
  await page.waitForTimeout(1500);
  expect((await plotImage(page)).equals(sampleB)).toBe(true);
  await focusEditor(page);
  expect(await selectedText(page)).toBe("*Radius* $sqrt(x^2+y^2)$");
  await page.keyboard.type("Replacement works");
  expect(await selectedText(page)).toBe("Replacement works");
});
