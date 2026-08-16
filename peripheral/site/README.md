# DoLogger 站点 / DoLogger Site

> `peripheral/site/` — GitHub Pages 落地页（Vue 3 + Vite + vue-i18n），
> 部署为 `https://nekolio.github.io/DoLogger/`。
> The landing site, deployed via GitHub Pages. Bilingual (zh-CN / EN).

## 结构 / Structure

- `src/App.vue` — 页面装配（三页 + 顶部功能栏 + 右侧导航 + 鼠标效果层）。
- `src/data.ts` — 数据层：GitHub API → localStorage 缓存 → 构建期烘焙 JSON →
  内置 v0.1.0 回退。释放资产按「前缀 + OS-arch 后缀」匹配（不依赖确切命名）。
- `src/composables/usePageNav.ts` — 轮播导航：一页一屏、一次手势一页、
  仅看方向（滚轮与触摸一致）；内层滚动容器优先原生滚动，硬锁定区吸收滚轮。
- `src/components/PageHero.vue` — 第一页：hero.svg + 徽章 + 标签 + 操作区
  （下载 / 资产筛选弹窗 / 实时 Star·Fork 统计 / 文档）。
- `src/components/FilterPopup.vue` — 资产筛选弹窗（1.5× 尺寸、相对触发按钮居中、
  完整文件名展示）。
- `src/components/PageDemo.vue` — 第二页「迁移演示」状态机：
  `idle → scrolling → overshoot → focusing → deleting → think → typing → done`。
  删除保留缩进（停在缩进边界并闪烁），输入完成后光标与高亮停留约 1.2s。
- `src/components/PageOverview.vue` — 第三页卡片墙。
  - PC：进入页面时卡片从屏外各方向飞入（Q 弹、错落时机），动画结束后交还
    CSS 3D 倾斜（±8°、角部光效强化）；无点击放大、无仪表动画；内容溢出在
    卡片内循环滚动（`useAutoLoopScroll`），架构管线为无缝横向跑马灯。
  - 移动端：折叠标题栈，进入时沿滑动相反方向逐条推入；唯一交互为点击展开
    「三卡窗口」（展开卡 + 上下邻卡撑满整页，FLIP 非线性动画）；展开卡内容
    循环滚动，hover 减速停止，滚轮有空间时卡片响应、到边界交还换页；支持
    陀螺仪倾斜。
- `src/components/CyberCursor.vue` — 鼠标效果层：拖尾长度随速度（停止时只剩
  光点）、离开页面淡出、按压缩放简洁克制。
- `public/assets/icons.svg` — 图标符号库（含心形/分支等）。
- `vite.config.js` — `base: './'`；`heroSingleSource` 插件使
  `docs/assets/hero.svg` 成为 hero 图像的唯一来源（dev 直接服务、build 发射进
  dist；`public/` 下不保存副本）。

## 资源约定 / Asset rules

- hero 图像唯一来源：`docs/assets/hero.svg`（由
  `peripheral/tools/hero-svg/hero_generator.py` 重新生成）。
- 源码中一律使用相对路径（`./assets/...`），不硬编码绝对地址。
- 站点构建：`bash peripheral/github/scripts/build-site.sh [OUT]`（构建 + 烘焙
  真实发布数据）；`bun run dev` 本地开发。

## 导航模型 / Navigation model

- PC：滚轮/键盘，一次手势一页；移动端：滑动，方向即一页（幅度/速度无关）。
- 内层滚动容器（终端、弹窗、第三页卡片）优先原生滚动；硬锁定区
  （`data-wheel-lock-hard`）吸收滚轮不换页。
- `prefers-reduced-motion` 下全部动画降级为静态终态。

## 说明 / Notes

- 代码注释与标识符为英文；界面文案双语（`src/i18n.ts`）。
- 本目录不参与 DoLogger 运行时构建；删除它不影响核心/CLI/插件（与
  `peripheral/tools/` 同约定）。

## 设计语言 / Design language

站点的视觉与交互语言围绕项目的三个支柱展开，所有细节统一遵循同一套
约束——克制、一致、可感知：

**三支柱的视觉表达 / Pillar motifs**
- **安全 / Security**：盾牌图标 + 青色/琥珀色光晕；信任相关的元素
  （审计链、信任分级、WORM）使用盾牌脉冲（`pulse-shield`）与 Ed25519
  链式强调。
- **插件化 / Plugin-ability**：插头/分层图标（`icon-plug`、`icon-layers`）；
  模块化内容（卡片、插件标签）共享统一的边框 + 低透明底填充语言。
- **性能 / Performance**：仪表/闪电图标（`icon-gauge`、`icon-zap`）；
  数值读数静态展示（无仪表动画），以克制的速度标注传达"快而不浮夸"。

**配色与字体 / Color & type**
- 品牌色沿用 hero.svg 的霓虹渐变（青 → 紫 → 品红），仅用于强调色、
  光标效果与品牌元素；正文用低饱和中性色，保证可读性。
- 数字、标签、资产名使用 JetBrains Mono 等宽字体；正文用系统无衬线。

**交互惯例 / Interaction conventions**
- 一次手势一页（滚轮/键盘/触摸方向一致）；内层滚动容器优先原生滚动，
  硬锁定区（`data-wheel-lock-hard`）吸收滚轮不换页。
- 可点击元素共享同一套 hover/按压语言（边框变色 + 柔和光晕）；卡片
  hover 时：取消上下渐隐（保证末端链接可读）、暂停自动滚动、滚轮接管
  卡片滚动。
- 自动滚动速度恒定（与内容量无关），hover 短暂宽限期后才暂停——让
  用户先看到滚动、再被交互接管。
- 移动端卡片展开/收起使用弹簧非线性动画（开快收缓），陀螺仪只作用于
  展开卡，收起即重置并脱离。
- `prefers-reduced-motion` 下全部动画降级为静态终态。

**主题 / Theme**
- 深色主题为赛博朋克终端风（扫描线、发光、霓虹），浅色主题为干净
  文档风；两套共享同一套语义色（accent/border/text 等 CSS 变量）。
