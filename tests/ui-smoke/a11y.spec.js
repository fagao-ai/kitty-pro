// A11y guards: native Playwright assertions plus an automated axe-core scan.
//
// The axe scan (U24) landed after the hand-written guards and is the only spec
// file that imports @axe-core/playwright; everything else stays plain DOM
// inspection so a broken axe release can never mask the manual checks.
const { test, expect } = require('@playwright/test');
const {
  LIMITS,
  VIEWS,
  gotoView,
  openApp,
  measureScroller,
  measureMainContent,
} = require('./helpers');

// helpers.js 不导出主题切换函数。UI 在 topbar 自带一个带可访问名的主题按钮
// （aria-label「切换深色外观」/「切换浅色外观」，同时带 aria-pressed），所以本
// 文件内封装一份切换；若该按钮不存在，则按任务要求跳过主题循环、只跑默认主题。
const THEME_TOGGLE = /切换(深色|浅色)外观/;
const THEMES = [
  { name: '浅色', dark: false },
  { name: '深色', dark: true },
];

const themeToggle = (page) => page.getByRole('button', { name: THEME_TOGGLE });

async function availableThemes(page) {
  // 没有可复用的主题切换能力时退化为只跑默认主题，不强行制造主题循环。
  return (await themeToggle(page).count()) ? THEMES : [THEMES[0]];
}

async function applyTheme(page, dark) {
  const toggle = themeToggle(page).first();
  if (!(await toggle.count())) return; // 退化为默认主题
  const root = page.locator('.proxy-app');
  const isDark = await root.evaluate((el) => el.classList.contains('theme-dark'));
  if (isDark !== dark) await toggle.click();
  await expect
    .poll(() => root.evaluate((el) => el.classList.contains('theme-dark')), {
      message: `主题未能切换到 ${dark ? '深色' : '浅色'}`,
    })
    .toBe(dark);
}

test.describe('a11y', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
  });

  test('每个视图的每个 button 都有可访问名（浅色 / 深色）', async ({ page }) => {
    for (const theme of await availableThemes(page)) {
      await applyTheme(page, theme.dark);
      for (const view of VIEWS) {
        await gotoView(page, view);
        const unnamed = await page.evaluate(() => {
          const describe = (el) => {
            const tag = el.tagName.toLowerCase();
            const id = el.id ? `#${el.id}` : '';
            const cls = typeof el.className === 'string' ? el.className.trim() : '';
            return `${tag}${id}${cls ? `.${cls.split(/\s+/).join('.')}` : ''}`;
          };
          return (
            Array.from(document.querySelectorAll('button'))
              .map((el, index) => {
                const label = (el.getAttribute('aria-label') || '').trim();
                const text = (el.textContent || '').trim();
                // 主判据按任务要求：textContent 或 aria-label 非空。title /
                // aria-labelledby 也计入可访问名来源，避免把纯图标按钮误判为无名。
                const title = (el.getAttribute('title') || '').trim();
                const labelledBy = (el.getAttribute('aria-labelledby') || '').trim();
                // Dioxus dev-overlay / toast 是框架噪声，不是 app 控件（与
                // measureScroller 的过滤一致）。
                const noise = el.closest('[id^="dx"], .dx-toast, .toast-viewport');
                return {
                  index,
                  selector: describe(el),
                  hasName: Boolean(label || text || title || labelledBy),
                  noise: Boolean(noise),
                };
              })
              .filter((entry) => !entry.noise && !entry.hasName)
              .map(({ index, selector }) => ({ index, selector }))
          );
        });
        expect(
          unnamed,
          `${theme.name}主题 / ${view}：以下 button 无可访问名（索引 / 选择器）：${JSON.stringify(unnamed)}`,
        ).toEqual([]);
      }
    }
  });

  test('所有 segmented-control button 都带 aria-pressed（浅色 / 深色）', async ({ page }) => {
    for (const theme of await availableThemes(page)) {
      await applyTheme(page, theme.dark);
      const missing = [];
      let checked = 0;
      for (const view of VIEWS) {
        await gotoView(page, view);
        const found = await page.evaluate(() =>
          Array.from(document.querySelectorAll('.segmented-control button')).map((el, index) => ({
            index,
            selector: `.segmented-control button:nth(${index})`,
            cls: typeof el.className === 'string' ? el.className : '',
            pressed: el.getAttribute('aria-pressed'),
          })),
        );
        checked += found.length;
        for (const entry of found) {
          if (entry.pressed === null) missing.push({ ...entry, view });
        }
      }
      expect(
        checked,
        `${theme.name}主题：未找到任何 segmented-control button，这条断言会失去意义`,
      ).toBeGreaterThan(0);
      expect(
        missing,
        `${theme.name}主题：以下 segmented-control button 缺少 aria-pressed：${JSON.stringify(missing)}`,
      ).toEqual([]);
    }
  });

  test('Tab 之后焦点元素有可见焦点样式（浅色 / 深色）', async ({ page }) => {
    for (const theme of await availableThemes(page)) {
      await applyTheme(page, theme.dark);
      // 切主题时点过按钮会留下焦点，先归位到文档开头，让第一次 Tab 从第一个
      // 可聚焦控件开始；键盘 Tab 会让 :focus-visible 命中，从而读到焦点环。
      await page.evaluate(() => {
        if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
      });
      await page.keyboard.press('Tab');
      const focus = await page.evaluate(() => {
        const el = document.activeElement;
        if (!el || el === document.body) return null;
        const style = getComputedStyle(el);
        return {
          tag: el.tagName,
          cls: typeof el.className === 'string' ? el.className : '',
          label: (el.getAttribute('aria-label') || el.textContent || '').trim().slice(0, 30),
          outlineStyle: style.outlineStyle,
          outlineWidth: style.outlineWidth,
        };
      });
      expect(focus, `${theme.name}主题：Tab 后没有任何元素获得焦点`).not.toBeNull();
      const visible = focus.outlineStyle !== 'none' || focus.outlineWidth !== '0px';
      expect(
        visible,
        `${theme.name}主题：Tab 后 ${focus.tag}.${focus.cls}（${focus.label}）缺少可见焦点环：outline=${focus.outlineStyle} ${focus.outlineWidth}`,
      ).toBe(true);
    }
  });

  test('320px 与 1600px 下无横向溢出（浅色 / 深色）', async ({ page }) => {
    for (const theme of await availableThemes(page)) {
      await applyTheme(page, theme.dark);
      for (const width of [320, 1600]) {
        await page.setViewportSize({ width, height: 800 });
        for (const view of VIEWS) {
          await gotoView(page, view);
          const metrics = await measureScroller(page);
          const inner = await measureMainContent(page);
          expect(
            metrics.horizontal,
            `${theme.name}主题 / ${view} / ${width}px：文档横向溢出 ${metrics.horizontal}px`,
          ).toBeLessThanOrEqual(LIMITS.horizontal);
          expect(
            inner,
            `${theme.name}主题 / ${view} / ${width}px：.main-content 内部溢出 ${inner}px`,
          ).toBeLessThanOrEqual(LIMITS.horizontal);
        }
      }
    }
  });

  // U5/U24：组件版 SegmentedControl 保留 role="group" + aria_pressed，但用
  // roving tabindex 让 Tab 只进入一次，并把方向键 / Home / End 变成焦点导航。
  // 设置页的「分流模式」是 .segmented-control.wide（3 个选项）；概览页还有一段
  // 手写的 .segmented-control，这里刻意按 .wide 精确定位组件版控件。
  const wideControl = (page) => page.locator('.segmented-control.wide');

  test('segmented-control 方向键 / Home / End 移动焦点且不误改选中（U5/U24）', async ({ page }) => {
    await gotoView(page, '设置');
    const control = wideControl(page);
    await expect(control, '设置页应存在组件版 .segmented-control.wide').toHaveCount(1);
    const buttons = control.locator('button');
    const total = await buttons.count();
    expect(total, `分流模式控件应有多个分段按钮，实际 ${total} 个`).toBeGreaterThan(1);

    // 初始状态：选中项 tabindex=0，其余 tabindex=-1，且恰有一个 aria-pressed。
    const initial = await control.evaluate((el) => {
      const list = Array.from(el.querySelectorAll('button'));
      return {
        pressedIndex: list.findIndex((b) => b.getAttribute('aria-pressed') === 'true'),
        pressedCount: list.filter((b) => b.getAttribute('aria-pressed') === 'true').length,
        tabbableIndex: list.findIndex((b) => b.getAttribute('tabindex') === '0'),
        negatives: list.filter((b) => b.getAttribute('tabindex') === '-1').length,
      };
    });
    expect(initial.pressedIndex, '应恰有一个 aria-pressed=true 的选中按钮').toBeGreaterThanOrEqual(0);
    expect(initial.pressedCount, 'aria-pressed=true 应恰好只有一个').toBe(1);
    expect(
      initial.tabbableIndex,
      `选中按钮（第 ${initial.pressedIndex} 个）应为唯一 tabindex=0，实际 tabindex=0 在第 ${initial.tabbableIndex} 个`,
    ).toBe(initial.pressedIndex);
    expect(
      initial.negatives,
      `未选中按钮应都是 tabindex=-1，期望 ${total - 1} 个，实际 ${initial.negatives} 个`,
    ).toBe(total - 1);

    // 聚焦选中按钮作为方向键起点。document::eval 的 .focus() 是异步落位的，
    // 所以每次按键后都用 expect.poll 等 document.activeElement 真正移动。
    await buttons.nth(initial.pressedIndex).focus();
    const focusIs = (index) =>
      control.evaluate(
        (el, i) => el.querySelectorAll('button')[i] === document.activeElement,
        index,
      );

    // ArrowRight：焦点循环移到下一项；手动激活模型下 aria_pressed 不应改变。
    const nextIndex = (initial.pressedIndex + 1) % total;
    await page.keyboard.press('ArrowRight');
    await expect
      .poll(() => focusIs(nextIndex), {
        message: `ArrowRight 后焦点应停在第 ${nextIndex} 个按钮（wrap 后的下一项）`,
        timeout: 5000,
      })
      .toBe(true);
    const pressedAfterRight = await control.evaluate((el) =>
      Array.from(el.querySelectorAll('button')).map((b) => b.getAttribute('aria-pressed')),
    );
    expect(
      pressedAfterRight.filter((p) => p === 'true').length,
      '方向键不应改变选中数量（手动激活：Enter/Space 才触发 onchange）',
    ).toBe(1);
    expect(
      pressedAfterRight[initial.pressedIndex],
      '方向键不应把选中误触发到相邻项，原选中项仍应为 aria-pressed=true',
    ).toBe('true');

    // Home：跳到首项；End：跳到末项。
    await page.keyboard.press('Home');
    await expect
      .poll(() => focusIs(0), { message: 'Home 后焦点应跳到第一个按钮', timeout: 5000 })
      .toBe(true);
    await page.keyboard.press('End');
    await expect
      .poll(() => focusIs(total - 1), {
        message: `End 后焦点应跳到最后一个按钮（第 ${total - 1} 个）`,
        timeout: 5000,
      })
      .toBe(true);

    // 全部焦点移动都不重排 DOM / 不改选中，roving tabindex 仍只认选中项。
    const finalState = await control.evaluate((el) => {
      const list = Array.from(el.querySelectorAll('button'));
      return {
        pressedIndex: list.findIndex((b) => b.getAttribute('aria-pressed') === 'true'),
        tabbableIndex: list.findIndex((b) => b.getAttribute('tabindex') === '0'),
        negatives: list.filter((b) => b.getAttribute('tabindex') === '-1').length,
      };
    });
    expect(finalState.pressedIndex, '方向键 / Home / End 全程不应改变选中项').toBe(initial.pressedIndex);
    expect(finalState.tabbableIndex, '选中按钮应始终保持 tabindex=0').toBe(initial.pressedIndex);
    expect(finalState.negatives, '未选中按钮应始终保持 tabindex=-1').toBe(total - 1);
  });

  test('Tab 进入 segmented-control 时只停一次（roving tabindex，U5/U24）', async ({ page }) => {
    await gotoView(page, '设置');
    const control = wideControl(page);
    await expect(control, '设置页应存在组件版 .segmented-control.wide').toHaveCount(1);
    const total = await control.locator('button').count();
    expect(total, `分流模式控件应有多个分段按钮，实际 ${total} 个`).toBeGreaterThan(1);

    const tabbable = control.locator('button[tabindex="0"]');
    const negatives = control.locator('button[tabindex="-1"]');
    await expect(
      tabbable,
      'roving tabindex：控件内只能有一个按钮在 Tab 序列里（tabindex=0）',
    ).toHaveCount(1);
    await expect(
      negatives,
      `其余 ${total - 1} 个按钮都应从 Tab 序列移除（tabindex=-1）`,
    ).toHaveCount(total - 1);

    // 从唯一的 tabbable 按钮按一次 Tab 就必须离开控件，不会在控件内再停一次。
    await tabbable.focus();
    await page.keyboard.press('Tab');
    await expect
      .poll(() => control.evaluate((el) => el.contains(document.activeElement)), {
        message: 'Tab 不应在 segmented-control 内停第二次（roving tabindex 生效）',
        timeout: 5000,
      })
      .toBe(false);
  });

  // U6/U11/U24：带 title 的图标按钮 / 只有图标没有可见文字的按钮，必须补
  // aria_label。这与上面「每个 button 都有可访问名」不同：那条允许 title /
  // text / labelledby 任一来源，这条严格要求图标按钮自带 aria_label，不重复。
  test('所有图标按钮（button[title] / 仅图标无文字）都有 aria_label（U6/U11/U24）', async ({ page }) => {
    for (const view of VIEWS) {
      await gotoView(page, view);
      const unlabelled = await page.evaluate(() => {
        const describe = (el) => {
          const tag = el.tagName.toLowerCase();
          const id = el.id ? `#${el.id}` : '';
          const cls = typeof el.className === 'string' ? el.className.trim() : '';
          return `${tag}${id}${cls ? `.${cls.split(/\s+/).join('.')}` : ''}`;
        };
        return Array.from(document.querySelectorAll('button'))
          .map((el, index) => {
            const noise = el.closest('[id^="dx"], .dx-toast, .toast-viewport');
            // 图标按钮 = 无可见文字（内容只有 svg / spinner）。带 title 的图标
            // 按钮同样落进这个集合；有可见文字的按钮由文本提供可访问名。
            const isIconOnly = (el.textContent || '').replace(/\s+/g, '').length === 0;
            const hasTitle = el.hasAttribute('title');
            const label = (el.getAttribute('aria-label') || '').trim();
            return {
              index,
              selector: describe(el),
              isIconOnly,
              hasTitle,
              hasLabel: Boolean(label),
              noise: Boolean(noise),
            };
          })
          .filter((entry) => !entry.noise && entry.isIconOnly && !entry.hasLabel)
          .map(({ index, selector, hasTitle }) => ({ index, selector, hasTitle }));
      });
      expect(
        unlabelled,
        `${view}：以下图标按钮（含 button[title]）缺少 aria_label（索引 / 选择器 / 是否有 title）：${JSON.stringify(unlabelled)}`,
      ).toEqual([]);
    }
  });
});

// ---------------------------------------------------------------------------
// U24｜axe-core 自动扫描（浅色 / 深色 × 全部视图）
// 规则集跑 axe 默认 best-practice + wcag2a/2aa 子集。任何 violation 都失败，
// 报告里带上 view / theme / rule id / 节点选择器，方便直接定位。
// ---------------------------------------------------------------------------
const { AxeBuilder } = require('@axe-core/playwright');

const AXE_TAGS = ['wcag2a', 'wcag2aa', 'best-practice'];
// color-contrast 在动画/过渡帧上偶发误报，reducedMotion 已开，保留断言；
// region（全页面 landmark）是产品设计取舍，暂不作门禁。
const AXE_DISABLED = ['region'];

// axe 扫描与轮询式渐进渲染有竞态：数据批次在 analyze 进行中插入新元素，
// 扫描会漏掉晚到的节点（偶发假阴性）或在半更新态上测量（假阳性，2026-11-05
// 首跑时「节点/深色」间歇失败即此）。扫描前等内容连续 800ms 无 DOM 变动。
async function waitForSettle(page, { quietMs = 800, timeoutMs = 20000 } = {}) {
  await page.evaluate(() => {
    window.__domMutations = 0;
    const observer = new MutationObserver(() => {
      window.__domMutations += 1;
    });
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      characterData: true,
    });
  });
  let last = await page.evaluate(() => window.__domMutations);
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    await page.waitForTimeout(quietMs);
    const now = await page.evaluate(() => window.__domMutations);
    if (now === last) return;
    last = now;
    if (Date.now() > deadline) return; // 页面确实在持续变动（如活动日志）：放弃等待直接扫描
  }
}

test.describe('axe 自动扫描（U24）', () => {
  for (const theme of THEMES) {
    for (const view of VIEWS) {
      test(`${view} 在${theme.name}主题下无 axe violation`, async ({ page }) => {
        await openApp(page);
        const toggle = themeToggle(page);
        const isDark = await toggle.getAttribute('aria-pressed');
        const wantDark = String(theme.dark);
        if (isDark !== wantDark && (await toggle.count())) {
          await toggle.click();
          await expect
            .poll(async () => await themeToggle(page).getAttribute('aria-pressed'))
            .toBe(wantDark);
        }
        await gotoView(page, view);
        await waitForSettle(page);
        const results = await new AxeBuilder({ page })
          .withTags(AXE_TAGS)
          .disableRules(AXE_DISABLED)
          .analyze();
        const digest = results.violations.map((v) => ({
          id: v.id,
          impact: v.impact,
          help: v.help,
          nodes: v.nodes.slice(0, 5).map((n) => `${n.target.join(' ')} :: ${(n.any[0]?.message || '').slice(0, 180)}`),
        }));
        expect(
          digest,
          `${view}/${theme.name}：axe 发现 ${results.violations.length} 类违规`,
        ).toEqual([]);
      });
    }
  }
});
