# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目状态

基于 GPUI 的原生 Markdown 桌面工作台(Markdown Workspace):左侧目录树、中心多文档编辑器、右侧大纲导航、可切换的 Markdown 预览、字体/字号/缩放设置(持久化)。全部 UI 文案为中文;Rust 代码注释保持中文。

## 构建与运行

```bash
cargo check          # 快速类型检查
cargo run            # 打开当前目录
cargo run -- <目录>  # 打开指定工作目录
cargo test           # 运行单元测试（model 模块的大纲提取等纯函数）
```

依赖 crate 全部为 git 依赖(`zed` 仓库的 `gpui`/`gpui_platform`、`longbridge/gpui-component`),首次构建慢。中文渲染依赖系统 CJK 字体(缺少时界面出现豆腐块,安装 `fonts-noto-cjk` 即可)。

## 架构

`main.rs` 打开窗口,**窗口第一层视图必须是 `Root`**(`gpui_component::Root`)包裹 `Workspace`——错误提示、对话框、通知层都依赖它。`Root` 提供的几个 `render_*_layer` 必须由应用显式挂载到视图树里,`Workspace` 在 `render` 末尾挂载了**对话框层**(`Root::render_dialog_layer`)和**通知层**(`Root::render_notification_layer`);漏挂对话框层会导致 `open_dialog` 的对话框不显示但不报错。

### 布局与面板(`workspace.rs`)

- `Workspace` 用 `DockSkin::dock_area` 组装三栏:左 = 目录树(`TreePanel`)、中央 = 编辑器(`EditorPanel`)、右 = 大纲(`OutlinePanel`)。状态栏含:打开目录、设置、dock 切换按钮。
- **面板状态流**:编辑器内容变化 → `EditorPanel::on_doc_changed`(dirty/大纲/防抖预览)→ `notify_others` → `Workspace::notify_panels` → 各面板 `cx.notify()`。
- 多文档由 `EditorPanel` 内部 `TabBar` 管理(不用 Dock 动态 tab:公开 API 无法激活已存在面板)。每个文档持有独立 `EditorState` + 预览 `TextViewState` 实体,切换 Tab 不丢光标/滚动。
- 编辑/预览公用中央区域:通过 `preview_mode` 切换。**预览 `TextView` 是 base 自绘组件,不继承父容器字体**,渲染时必须显式 `.font_family(cx.theme().mono_font_family.clone())`。
- 快捷键(`Workspace::init` 绑定):`Ctrl+S` 保存、`Ctrl+Shift+V` 切换右 dock、`Ctrl+=`/`Ctrl--` 整体缩放。

### 设置框架(`settings.rs`)——如何新增一个设置组

应用配置是个 serde 结构,持久化到 `$XDG_CONFIG_HOME/MarkSpace/config.json`:

- `AppConfig { font: FontSettings, zoom: ZoomSettings }`,所有结构 `#[serde(default)]`——缺失字段/旧配置自动回落默认,向前兼容。
- 设置对话框 `SettingsDialog` 是独立 `Entity`,左侧分组导航(`SettingsGroup` 枚举)+ 右侧表单。
- **新增设置组的四步**:① `AppConfig` 加字段;② `SettingsGroup` 枚举加变体 + `ALL` 数组 + `label()`;③ `SettingsDialog::render` 的 match 加分支、新写一个表单渲染函数;④ `Workspace::apply_theme` 把新配置应用到运行时。

**字体/缩放生效机制(核心,改字号/字体必须走它)**:组件字号/字体全部来自全局主题,不读 rem。修改 `Theme::global_mut(cx)` 的 `font_family`/`mono_font_family`/`font_size`/`mono_font_size` + `Theme::sync_base(cx)` + `window.set_rem_size(...)` + `cx.notify()` 即可全局生效。`Workspace` 里的 `apply_theme(cx, content_zoom)` 是唯一入口;`apply_settings`(启动/提交)、`apply_pending`(设置即时预览/取消回滚)、`zoom_content`(Ctrl+滚轮)都走它。

- `font_size` = 配置界面字号 × `ui_zoom`;`mono_font_size` = 配置编辑区字号 × `ui_zoom` × `content_zoom`(内容区局部缩放已并入主题值,**不要在渲染元素上再乘一次**)。
- **预览代码块字体**:阅读 `tokens.typography.mono_md.size`(经 `sync_base` 同步为 `mono_font_size`)。正因为把它并入 `mono_font_size`,代码块才自动跟随字号/缩放设置。
- 缩放系数持久化在配置里,`zoom_ui`/`zoom_content` 修改后立即 `save_config`。

### 对话框用法(gpui-component,全是踩过的坑)

- **必须用命令式 `window.open_dialog(cx, |dialog, _, cx| …)`**(`gpui_component::WindowExt`)。把 `Dialog::new(cx)` 直接渲染进视图树(`.when(...)`)是错的:关闭机制绕过了 Root 层,对话框会反复弹/关不掉。两个对话框(设置、未保存确认)都走 `open_dialog`。
- `Dialog::button_props(...)` 是**整体赋值**,`on_ok`/`on_cancel` 必须放进 `DialogButtonProps` 链,`on_close` 在 `button_props()` 之后设置,否则被覆盖。
- builder 闭包是 `Fn`(每帧重跑):**嵌套闭包的捕获变量需各自 `clone()`**(Entity/WeakEntity 非 `Copy`)。
- 取消语义:`on_cancel`(取消按钮/遮罩)/`on_close`(Esc/右上角 X)都调用 `SettingsDialog::rollback`(回滚到最近提交;确定后 `committed` 推进,回滚无变化)。

## gpui 与 gpui-component API 坑

- 组件事件回调(`Button::on_click`、`PopupMenuItem::on_click` 等)收到 `&mut App`,要改视图状态用 `cx.listener(|this, event, window, cx| …)`;回调里 `window` 是 `&mut Window`,但订阅(`cx.subscribe`)回调里**没有 window**。
- `Button::new(id).primary()` 需要 `use gpui_component::button::ButtonVariants`;`.selected(bool)` 需要 `Selectable` trait。
- `cx.subscribe` 返回的 `Subscription` **必须持有**(存进字段),否则订阅立即失效。
- `window.on_mouse_event` **只能在 paint 阶段调用**(debug 构建断言 panic)。项目用自定义零尺寸元素 `CaptureWheelZoom`(实现 `Element`,在 `paint` 里注册)在 **Capture 阶段**拦截 Ctrl+滚轮缩放——因为输入框/编辑器组件内部滚轮处理会 `stop_propagation`,Bubble 阶段收不到。
- `overflow_y_scroll` 等 overflow 便捷方法属于 `StatefulInteractiveElement`(元素需先 `.id(...)`),纯 `Div` 没有。
- `window.text_system()` 返回借用;跨线程枚举字体需用 `cx.text_system().clone()`(Arc)。
- `ThemeMode` 不是 `Display`,用 `{:?}` 格式化。