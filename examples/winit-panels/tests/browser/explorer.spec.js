import { expect, test } from "@playwright/test";

const errors = new WeakMap();
test.beforeEach(async ({ page }) => {
  const messages = [];
  page.on("pageerror", error => messages.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") messages.push(message.text());
  });
  errors.set(page, messages);
  await page.goto("/");
  await expect(page.locator("#status")).toBeHidden();
  await expect.poll(async () => (await page.locator("canvas").screenshot()).length).toBeGreaterThan(15000);
});
test.afterEach(async ({ page }) => expect(errors.get(page)).toEqual([]));

async function controlImage(page, index) {
  const box = await page.locator("canvas").boundingBox();
  return page.screenshot({ clip: { x: box.x + box.width - 262, y: box.y + 153 + index * 65, width: 222, height: 32 } });
}
async function clickControl(page, index) {
  const box = await page.locator("canvas").boundingBox();
  await page.locator("canvas").click({ position: { x: box.width - (index === 0 ? 84 : 180), y: 170 + index * 65 } });
}

test("pointer and keyboard cycle the same domain scope", async ({ page }) => {
  const initial = await controlImage(page, 0);
  await clickControl(page, 0);
  await page.mouse.move(4, 4);
  await expect.poll(async () => (await controlImage(page, 0)).equals(initial)).toBe(false);
  const next = await controlImage(page, 0);
  await page.keyboard.press("1");
  await expect.poll(async () => (await controlImage(page, 0)).equals(next)).toBe(false);
  await page.keyboard.press("1");
  await expect.poll(async () => (await controlImage(page, 0)).equals(initial)).toBe(true);
});

test("all controls render, and narrow views gain scroll space", async ({ page }) => {
  for (const index of [1, 2, 3, 4, 5, 6, 7]) {
    const before = await controlImage(page, index);
    await clickControl(page, index);
    await expect.poll(async () => (await controlImage(page, index)).equals(before)).toBe(false);
  }
  await clickControl(page, 5); // Empty panel becomes a physical hole.
  await page.setViewportSize({ width: 940, height: 900 });
  await expect.poll(async () => (await page.locator("canvas").boundingBox()).height).toBe(1100);
  await page.setViewportSize({ width: 760, height: 900 });
  await expect.poll(async () => (await page.locator("canvas").boundingBox()).height).toBe(1600);
  await page.locator("canvas").screenshot({ path: "test-results/narrow.png" });
  await page.setViewportSize({ width: 1280, height: 900 });
  await expect.poll(async () => (await page.locator("canvas").boundingBox()).height).toBe(900);
});
