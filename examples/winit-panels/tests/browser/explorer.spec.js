import { expect, test } from "@playwright/test";

const snapshot = page => page.evaluate(() => window.panelExplorerSnapshot());
const errors = new WeakMap();
test.beforeEach(async ({ page }) => {
  test.setTimeout(60_000);
  const messages = [];
  page.on("pageerror", error => messages.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") messages.push(message.text());
  });
  errors.set(page, messages);
  await page.goto("/");
  await expect.poll(() => snapshot(page).then(s => s.controls.length).catch(() => 0), { timeout: 30_000 }).toBeGreaterThan(8);
  await expect(page.locator("#status")).toBeHidden();
  // A cold software WebGPU adapter can take several seconds to initialize.
  await expect.poll(() => page.locator("canvas").evaluate(e => e.width === Math.round(e.clientWidth * devicePixelRatio) && e.height === Math.round(e.clientHeight * devicePixelRatio)), { timeout: 30_000 }).toBe(true);
  await expect.poll(async () => (await page.locator("canvas").screenshot()).length, { timeout: 30_000 }).toBeGreaterThan(15000);
});
test.afterEach(async ({ page }) => expect(errors.get(page)).toEqual([]));

async function clickControl(page, index, item = null) {
  const state = await snapshot(page);
  const r = state.controls.find(c => c.id === `control-${index}` && c.item === item);
  expect(r).toBeTruthy();
  await page.locator("canvas").click({ position: { x: r.x + r.width / 2, y: r.y + r.height / 2 } });
}

test("radio groups select domain, title, and legend scopes independently", async ({ page }) => {
  await clickControl(page, 0, "figure");
  await clickControl(page, 2, "panel");
  await clickControl(page, 3, "panel");
  await expect.poll(() => snapshot(page).then(s => [s.yScope, s.titleScope, s.legendScope])).toEqual([2, 0, 0]);
  await clickControl(page, 3, "figure");
  await clickControl(page, 3, "figure"); // Selecting the current choice must not cycle it.
  await expect.poll(() => snapshot(page).then(s => [s.yScope, s.titleScope, s.legendScope])).toEqual([2, 0, 2]);
  await clickControl(page, 2, "region");
  await page.keyboard.press("Tab");
  await expect.poll(() => snapshot(page).then(s => s.focus)).toEqual({ id: "control-3", item: "figure" });
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => snapshot(page).then(s => s.legendScope)).toBe(1);
  await page.keyboard.press("1");
  await expect.poll(() => snapshot(page).then(s => s.yScope)).toBe(0);
});

test("product states, presets, and checkboxes use direct widget choices", async ({ page }) => {
  await clickControl(page, 1);
  await expect.poll(() => snapshot(page).then(s => s.outer)).toBe(false);
  await page.keyboard.press("Space");
  await expect.poll(() => snapshot(page).then(s => s.outer)).toBe(true);
  await clickControl(page, 4);
  await clickControl(page, 6);
  await expect.poll(() => snapshot(page).then(s => [s.legendBottom, s.overlay])).toEqual([true, true]);
  await clickControl(page, 5, "hole");
  await clickControl(page, 5, "hole");
  await expect.poll(() => snapshot(page).then(s => s.missing)).toBe(2);
  await page.keyboard.press("ArrowLeft");
  await expect.poll(() => snapshot(page).then(s => s.missing)).toBe(1);
  await clickControl(page, 5, "data");
  await expect.poll(() => snapshot(page).then(s => s.missing)).toBe(0);
  await clickControl(page, 7, "units");
  await expect.poll(() => snapshot(page).then(s => [s.preset, s.yScope])).toEqual([2, 1]);
  await page.keyboard.press("ArrowUp");
  await expect.poll(() => snapshot(page).then(s => [s.preset, s.yScope])).toEqual([1, 0]);
  await clickControl(page, 7, "sales");
  await expect.poll(() => snapshot(page).then(s => [s.preset, s.yScope])).toEqual([0, 1]);
});

test("all choices remain visible through narrow and short layouts", async ({ page }) => {
  await clickControl(page, 5, "hole");
  await clickControl(page, 6);
  for (const [width, height, canvasHeight] of [[1280, 780, 780], [940, 900, 1100], [760, 900, 1600], [1280, 900, 900]]) {
    await page.setViewportSize({ width, height });
    await expect.poll(() => snapshot(page).then(s => s.size)).toEqual([width, canvasHeight]);
    const state = await snapshot(page);
    for (const control of state.controls) {
      expect(control.x).toBeGreaterThanOrEqual(0);
      expect(control.y).toBeGreaterThanOrEqual(0);
      expect(control.x + control.width).toBeLessThanOrEqual(width);
      expect(control.y + control.height).toBeLessThanOrEqual(canvasHeight);
    }
    await clickControl(page, 7, "units");
    await expect.poll(() => snapshot(page).then(s => s.preset)).toBe(2);
    await clickControl(page, 7, "sales");
    await expect.poll(() => snapshot(page).then(s => s.preset)).toBe(0);
  }
  await page.locator("canvas").screenshot({ path: "test-results/direct-choices.png" });
});
