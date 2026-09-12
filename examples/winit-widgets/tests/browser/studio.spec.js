import { expect, test } from '@playwright/test';

const snapshot = page => page.evaluate(() => window.widgetStudioSnapshot());
const focus = page => snapshot(page).then(s => s.focus);
async function control(page, id, item = null) {
  const state = await snapshot(page);
  const r = state.controls.find(c => c.id === id && c.item === item);
  expect(r).toBeTruthy();
  return r;
}
async function click(page, id, item = null) {
  const r = await control(page, id, item);
  await page.locator('canvas').click({ position: { x: r.x + r.width / 2, y: r.y + r.height / 2 } });
}
async function selectAll(page) {
  const mac = await page.evaluate(() => /^(Mac|iP)/.test(navigator.platform));
  await page.keyboard.press(`${mac ? 'Meta' : 'Control'}+a`);
}
async function clipboard(page, type, text = '') {
  return page.evaluate(({ type, text }) => {
    const data = new DataTransfer();
    if (type === 'paste') data.setData('text/plain', text);
    document.activeElement.dispatchEvent(new ClipboardEvent(type, { clipboardData: data, bubbles: true, cancelable: true }));
    return data.getData('text/plain');
  }, { type, text });
}
async function compose(page, type, data) {
  await page.locator('input').evaluate((e, { type, data }) => e.dispatchEvent(new CompositionEvent(type, { data, bubbles: true })), { type, data });
}
const errors = new WeakMap();
test.beforeEach(async ({ page }) => {
  const found = [];
  page.on('pageerror', e => found.push(e.message));
  page.on('console', m => { if (m.type() === 'error') found.push(m.text()); });
  errors.set(page, found);
  await page.goto('/');
  await expect.poll(() => snapshot(page).then(s => s.controls.length).catch(() => 0), { timeout: 30_000 }).toBeGreaterThan(8);
  await expect(page.locator('#status')).toBeHidden();
  await expect.poll(() => page.locator('canvas').evaluate(e => e.width === Math.round(e.clientWidth * devicePixelRatio) && e.height === Math.round(e.clientHeight * devicePixelRatio)), { timeout: 30_000 }).toBe(true);
  await expect.poll(async () => (await page.locator('canvas').screenshot()).length, { timeout: 30_000 }).toBeGreaterThan(15000);
});
test.afterEach(async ({ page }) => expect(errors.get(page)).toEqual([]));

test('keyboard entry, group tab models, and exit in both directions', async ({ page }) => {
  await page.locator('#before').focus();
  await page.keyboard.press('Tab');
  await expect.poll(() => focus(page)).toEqual({ id: 'lock', item: null });
  for (const item of ['grid', 'points', 'annotations']) {
    await page.keyboard.press('Tab');
    await expect.poll(() => focus(page)).toEqual({ id: 'layers', item });
  }
  await page.keyboard.press('Tab');
  await expect.poll(() => focus(page)).toEqual({ id: 'palette', item: 'ocean' });
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => snapshot(page).then(s => s.palette)).toBe('sunset');
  for (const id of ['size', 'opacity', 'title', 'source', 'reset']) {
    await page.keyboard.press('Tab');
    await expect.poll(() => focus(page)).toEqual({ id, item: null });
  }
  await page.keyboard.press('Tab');
  await expect(page.locator('#after')).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect.poll(() => focus(page)).toEqual({ id: 'reset', item: null });
  for (let i = 0; i < 9; i++) await page.keyboard.press('Shift+Tab');
  await expect.poll(() => focus(page)).toEqual({ id: 'lock', item: null });
  await page.keyboard.press('Shift+Tab');
  await expect(page.locator('#before')).toBeFocused();
});

test('check groups, radio selection, lock and reset bind to application values', async ({ page }) => {
  await click(page, 'layers', 'grid');
  await expect.poll(() => snapshot(page).then(s => s.layers)).toEqual(['annotations', 'points']);
  await page.keyboard.press('Space');
  await expect.poll(() => snapshot(page).then(s => s.layers)).toEqual(['annotations', 'grid', 'points']);
  await click(page, 'palette', 'mono');
  await expect.poll(() => snapshot(page).then(s => s.palette)).toBe('mono');
  await click(page, 'lock');
  await expect.poll(() => snapshot(page).then(s => s.locked)).toBe(true);
  await click(page, 'layers', 'points');
  await expect.poll(() => snapshot(page).then(s => s.layers)).toContain('points');
  await click(page, 'reset');
  await expect.poll(() => snapshot(page).then(s => [s.locked, s.palette])).toEqual([false, 'ocean']);
  expect((await snapshot(page)).pan).toEqual([0, 0]);
});

test('slider preview, outside capture, keyboard commit, and plot panning stay separate', async ({ page }) => {
  const r = await control(page, 'opacity');
  const box = await page.locator('canvas').boundingBox();
  await page.mouse.move(box.x + r.x + 30, box.y + r.y + r.height / 2);
  await page.mouse.down();
  await expect.poll(() => snapshot(page).then(s => s.opacity)).toBeLessThan(.3);
  expect((await snapshot(page)).committedOpacity).toBe(.8);
  await page.mouse.move(box.x + r.x + 150, box.y - 10, { steps: 6 });
  await page.mouse.up();
  await expect.poll(() => snapshot(page).then(s => s.committedOpacity === s.opacity)).toBe(true);
  expect((await snapshot(page)).pan).toEqual([0, 0]);
  await click(page, 'size');
  await page.keyboard.press('End');
  await expect.poll(() => snapshot(page).then(s => s.size)).toBe(15);
  await page.keyboard.press('ArrowLeft');
  await expect.poll(() => snapshot(page).then(s => s.size)).toBe(14);
  const currentBox = await page.locator('canvas').boundingBox();
  await page.mouse.move(currentBox.x + 200, currentBox.y + 300);
  await page.mouse.down();
  await page.mouse.move(currentBox.x + 245, currentBox.y + 325, { steps: 5 });
  await page.mouse.up();
  await expect.poll(() => snapshot(page).then(s => s.pan)).toEqual([45, 25]);
  await click(page, 'size');
  const beforeScroll = await page.evaluate(() => scrollY);
  await page.keyboard.press('Space');
  await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(beforeScroll);

});

test('text editing, clipboard, Typst validation, history, and composition', async ({ page }) => {
  await click(page, 'title');
  await expect(page.locator('input')).toBeFocused();
  await selectAll(page);
  await page.keyboard.type('Caf');
  await compose(page, 'compositionstart', '');
  await compose(page, 'compositionupdate', 'é');
  expect((await snapshot(page)).title).toBe('Caf');
  await compose(page, 'compositionend', 'é');
  await page.locator('input').evaluate(e => e.dispatchEvent(new InputEvent('input', { data: 'é', inputType: 'insertCompositionText', bubbles: true })));
  await expect.poll(() => snapshot(page).then(s => s.title)).toBe('Café');
  await selectAll(page);
  expect(await clipboard(page, 'copy')).toBe('Café');
  expect(await clipboard(page, 'cut')).toBe('Café');
  await clipboard(page, 'paste', 'A\nB');
  await expect.poll(() => snapshot(page).then(s => s.title)).toBe('AB');
  const modifier = await page.evaluate(() => /^(Mac|iP)/.test(navigator.platform) ? 'Meta' : 'Control');
  await page.keyboard.press(`${modifier}+z`);
  await expect.poll(() => snapshot(page).then(s => s.title)).toBe('');
  await page.keyboard.press(`${modifier}+Shift+z`);
  await expect.poll(() => snapshot(page).then(s => s.title)).toBe('AB');
  await click(page, 'source');
  await selectAll(page);
  await page.keyboard.type('*Browser* $sqrt(x^2+y^2)$');
  await expect.poll(() => snapshot(page).then(s => s.acceptedSource)).toBe('*Browser* $sqrt(x^2+y^2)$');
  await selectAll(page);
  await page.keyboard.type('$sqrt(x');
  await expect.poll(() => snapshot(page).then(s => s.invalid)).toBe(true);
  expect((await snapshot(page)).acceptedSource).toBe('*Browser* $sqrt(x^2+y^2)$');
  await page.keyboard.press('Enter');
  await expect(page.locator('input')).toBeFocused();
  await click(page, 'reset');
  await expect.poll(() => snapshot(page).then(s => [s.source, s.invalid])).toEqual(['*Radius* $sqrt(x^2+y^2)$', false]);
});

test('late composition does not cross focus targets and controls survive resizing', async ({ page }) => {
  await click(page, 'title');
  await page.keyboard.press('End');
  await compose(page, 'compositionstart', '');
  await compose(page, 'compositionupdate', 'x');
  await click(page, 'source');
  await compose(page, 'compositionend', 'x');
  expect((await snapshot(page)).source).toBe('*Radius* $sqrt(x^2+y^2)$');
  expect((await snapshot(page)).title).toBe('Signal and variation');
  await page.locator('canvas').screenshot({ path: 'test-results/studio-browser.png' });
  await page.setViewportSize({ width: 960, height: 930 });
  await expect.poll(() => control(page, 'layers', 'grid').then(r => r.x)).toBe(604);
  await click(page, 'layers', 'grid');
  await expect.poll(() => snapshot(page).then(s => s.layers)).not.toContain('grid');
});


test.describe('high-density canvas', () => {
  test.use({ deviceScaleFactor: 2, viewport: { width: 1180, height: 960 } });
  test('initial layout and pointer coordinates agree at DPR 2', async ({ page }) => {
    expect(await page.evaluate(() => devicePixelRatio)).toBe(2);
    const canvas = page.locator('canvas');
    const dimensions = await canvas.evaluate(e => ({ backing: [e.width, e.height], css: [e.clientWidth, e.clientHeight] }));
    expect(dimensions.backing).toEqual(dimensions.css.map(v => v * 2));
    await click(page, 'layers', 'grid');
    await expect.poll(() => snapshot(page).then(s => s.layers)).not.toContain('grid');
    await click(page, 'title');
    await selectAll(page);
    await page.keyboard.type('Retina title');
    await expect.poll(() => snapshot(page).then(s => s.title)).toBe('Retina title');
    await canvas.screenshot({ path: 'test-results/studio-dpr2.png' });
  });
});


test('lost pointer capture cancels slider preview without a commit', async ({ page }) => {
  const canvas = page.locator('canvas');
  await canvas.evaluate(e => e.addEventListener('pointerdown', event => {
    e.dataset.capturedPointer = String(event.pointerId);
  }));
  await canvas.evaluate(e => e.addEventListener('gotpointercapture', () => {
    e.dataset.captureStarted = 'true';
  }));
  const r = await control(page, 'opacity');
  const box = await canvas.boundingBox();
  await page.mouse.move(box.x + r.x + 30, box.y + r.y + r.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + r.x + 31, box.y + r.y + r.height / 2);
  await expect.poll(() => canvas.evaluate(e => e.dataset.captureStarted === 'true' && e.hasPointerCapture(Number(e.dataset.capturedPointer)))).toBe(true);
  await expect.poll(() => snapshot(page).then(s => s.opacity)).toBeLessThan(.3);
  const preview = (await snapshot(page)).opacity;
  await canvas.evaluate(e => e.releasePointerCapture(Number(e.dataset.capturedPointer)));
  await page.mouse.move(box.x + r.x + 100, box.y + r.y + r.height / 2);
  await expect.poll(() => snapshot(page).then(s => s.lastAction)).toBe('Slider cancelled: CaptureLost');
  await page.mouse.up();
  expect((await snapshot(page)).opacity).toBe(preview);
  expect((await snapshot(page)).committedOpacity).toBe(.8);
});
