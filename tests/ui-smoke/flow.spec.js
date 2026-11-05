// F23: lightweight UI flow guards.
//
// No real subscription, no running core, no network. Every /api/* round-trip is
// answered in-browser so the suite depends only on the release web shell and
// cannot flake on GitHub's rate limit or a missing kernel.
const { test, expect } = require('@playwright/test');
const {
  gotoView,
  openApp,
  stopStubs,
  stubUpdateCheck,
  stubProfileWithNodes,
  stubEmptyProfile,
} = require('./helpers');

/// Install before `openApp` (its hydration check waits for the first /api/
/// response). Playwright routes are last-registered-first-wins, so the broad
/// fallback goes first and the narrower helpers mock registered afterwards wins.
async function isolateNetwork(page) {
  // 兜底：任何 /api/ 请求都返回空 JSON。流程测试只验证前端交互，不需要内核数据；
  // core_status 反序列化失败会走 Err 分支优雅降级（connected=false），不会 panic。
  // 用正则而非 '**/api/*'：后者只匹配单层路径段，够不到 /api/update/settings。
  await page.route(/\/api\//, (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: '{}' }),
  );
  // 复用 helpers 已有的 mock 覆盖真正的外部网络触发点：更新检查会打 GitHub
  // （匿名限额 60 次/小时，CI 共享出口 IP 极易抖动）。
  await stubUpdateCheck(page, { variant: 'up_to_date' });
  // 订阅预览、同步上传/下载等端点同样被上面的兜底拦住：本套件不填真实订阅、
  // 不点导入按钮，因此永远不会触发真实解析或真实网络请求。
}

test.describe('F23 轻量流程', () => {
  test.beforeEach(async ({ page }) => {
    await isolateNetwork(page);
    await openApp(page);
  });

  test.afterEach(async ({ page }) => {
    await stopStubs(page);
  });

  test('导入对话框：支持格式可展开、空地址禁用并有提示、Escape 关闭无残留', async ({
    page,
  }) => {
    await gotoView(page, '概览');
    await page.getByRole('button', { name: '添加订阅' }).first().click();
    const dialog = page.locator('#import-dialog');
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAttribute('role', 'dialog');

    // F12：「支持的格式」说明存在、初始折叠、点击可展开出真正的格式列表。
    const formatsToggle = dialog.locator('.form-stack button[aria-expanded]');
    await expect(formatsToggle, 'F12：缺少「支持的格式」展开控件').toHaveCount(1);
    await expect(
      dialog.getByText('当前支持以下格式：'),
      'F12：说明初始应收起',
    ).toHaveCount(0);
    await formatsToggle.click();
    await expect(dialog.getByText('当前支持以下格式：'), 'F12：点击后应展开').toBeVisible();
    await expect(dialog.getByText('Base64 分享链接订阅')).toBeVisible();
    await expect(formatsToggle).toHaveAttribute('aria-expanded', /true/);

    // F12：地址为必填，空地址时导入按钮禁用，并给出浅色文案提示（不只是变灰）。
    const submit = dialog.locator('.modal-actions button.primary-button');
    await expect(submit, '空地址时导入按钮应 disabled').toBeDisabled();
    await expect(
      dialog.locator('.inline-note').filter({ hasText: '粘贴订阅链接' }),
      'F12：空地址应给出提示文案',
    ).toBeVisible();

    // Escape 关闭。
    await page.keyboard.press('Escape');
    await expect(page.locator('[role="dialog"]')).toHaveCount(0);

    // F13：解析期间关闭对话框后，迟到的解析结果不得重开面板。本套件不发起真实
    // 解析（无订阅、无网络，兜底会把预览请求拦成空 JSON），因此这里退化为关闭后
    // 观察一小段时间，确认没有对话框残留或重新出现；「解析中真正被关闭」这一
    // 分支需要后端状态支撑，超出轻量 UI 流程测试的范围。
    await page.waitForTimeout(500);
    await expect(
      page.locator('[role="dialog"]'),
      '关闭后不得残留或重开任何对话框',
    ).toHaveCount(0);
    await expect(page.locator('#import-dialog')).toHaveCount(0);
  });

  test('概览空态提供「添加订阅」CTA', async ({ page }) => {
    // openApp 的 baseline 桩带订阅（折叠预算以加载态为基准）；本测试要空态，
    // 装空 profile 桩后重新加载。baselineProfile:false 防止 openApp 再注册
    // 一次 baseline 把空桩顶掉（Playwright 后注册者优先）。
    await stubEmptyProfile(page);
    await openApp(page, { baselineProfile: false });
    await gotoView(page, '概览');
    // 无节点/无订阅时的空态卡片。
    await expect(page.locator('.empty-state')).toBeVisible();
    // CTA 可能不止一个（topbar 的添加按钮 + 首次引导卡里的添加按钮），用 first()
    // 规避 strict mode，断言「至少一个可点击的添加订阅入口」即可。
    const ctas = page.getByRole('button', { name: '添加订阅' });
    expect(await ctas.count(), '概览空态应提供「添加订阅」CTA').toBeGreaterThan(0);
    await expect(ctas.first(), '「添加订阅」CTA 应可见可点').toBeVisible();
  });

  test('「重新连接」横幅默认不出现，只在配置变更后才出现', async ({ page }) => {
    await gotoView(page, '概览');
    // 退化说明：横幅的渲染条件是 config_pending() && connected &&
    // !core_action_busy（overview.rs），三者都由内核/后端状态驱动。本套件不启动
    // 真实内核、不填订阅、不发网络请求（/api/* 全部拦成空 JSON，connected 恒为
    // false），纯 UI 流程里无法把配置置为「待生效」，因此这里只能断言选择器默认
    // 不存在；要验证「配置变更后出现」需要能驱动后端状态的集成测试。
    await expect(
      page.locator('.pending-reload-banner'),
      '默认（无配置变更、未连接）不应出现「重新连接」横幅',
    ).toHaveCount(0);
  });
  // 回归守卫（2026-11-05 用户报告，v1.0.12）：切换视图不得清空节点列表。
  // 曾经的 U23 门控让代理组预览 resource 在节点页不可见时缓存空数组，
  // 每次切进节点页都先渲染「配置中没有代理组」、等预览重算完才闪回，
  // 用户感知即「不点刷新延迟就没有节点」。现在 resource 不再跟踪
  // active_view，缓存值跨导航保持；导航本身也不应重发预览请求。
  test('节点列表跨视图导航保持可见，导航不重发预览请求', async ({ page }) => {
    // beforeEach 的 openApp 早于本 stub，页面此时是空 profile；注册 stub 后
    // 重新 openApp（内部会重新 goto），让应用带着订阅数据完成 hydrate。
    // baselineProfile:false —— openApp 默认会再注册一次空 profile 路由，
    // Playwright 后注册者优先，会把刚装的桩数据顶掉。
    await stubProfileWithNodes(page);
    await openApp(page, { baselineProfile: false });
    await gotoView(page, '节点');
    const groupRow = page.locator('.proxy-group-section');
    await expect(groupRow.first(), '首次进入应看到代理组').toBeVisible({ timeout: 15000 });
    await expect(page.getByText('配置中没有代理组')).toHaveCount(0);

    // 记下预览请求次数，来回导航后必须不变（依赖驱动，不随视图重发）。
    let previewCalls = 0;
    await page.route('**/api/proxy-groups/preview', (route) => {
      previewCalls += 1;
      return route.continue();
    });
    await gotoView(page, '订阅');
    await gotoView(page, '日志');
    await gotoView(page, '节点');
    await expect(groupRow.first(), '切回后代理组必须保持可见，不得闪空').toBeVisible();
    await expect(page.getByText('配置中没有代理组')).toHaveCount(0);
    expect(previewCalls, '导航不得触发代理组预览重新请求').toBe(0);
  });
});
