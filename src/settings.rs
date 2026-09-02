//! 设置框架：配置模型 + 持久化 + 设置对话框（分组导航）。
//!
//! 新增设置组的步骤：
//! 1. 给 `AppConfig` 加字段（serde 默认值保证旧配置向前兼容）；
//! 2. `SettingsGroup` 枚举加变体 + `ALL` 数组 + `label()`；
//! 3. `SettingsDialog::render` 的 match 加一个分支；
//! 4. `Workspace` 里应用该配置到运行时。

use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, WeakEntity, Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{InputEvent, InputState, NumberInput};
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use gpui_component::{ActiveTheme, Selectable, Sizable, h_flex, v_flex};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---- 配置模型 --------------------------------------------------------------

/// 全部应用配置：每个设置组一个字段，serde(default) 容错缺失字段。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub font: FontSettings,
    pub zoom: ZoomSettings,
    // 未来：pub theme: ThemeSettings, pub behavior: BehaviorSettings, …
}

/// 字体设置组。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontSettings {
    /// 界面普通字体家族名（".SystemUIFont" = 系统默认 UI 字体）。
    pub ui_font_family: String,
    /// 编辑器等宽字体家族名。
    pub mono_font_family: String,
    /// 界面基准字号（px）。
    pub ui_font_size: f32,
    /// 编辑区基准字号（px）。
    pub mono_font_size: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self {
            // 出厂默认字体统一为 YaHei Consolas Hybrid（界面与等宽共用）
            ui_font_family: "YaHei Consolas Hybrid".into(),
            mono_font_family: "YaHei Consolas Hybrid".into(),
            // 界面字号默认 25px，编辑区字号默认 12px
            ui_font_size: 25.0,
            mono_font_size: 12.0,
        }
    }
}

/// 缩放设置（Ctrl+/- 整体缩放、Ctrl+滚轮编辑区缩放）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoomSettings {
    /// 整体界面缩放系数（0.5–3.0）。
    pub ui_zoom: f32,
    /// 编辑/预览区缩放系数（0.5–3.0）。
    pub content_zoom: f32,
}

impl Default for ZoomSettings {
    fn default() -> Self {
        Self {
            ui_zoom: 1.0,
            content_zoom: 1.0,
        }
    }
}

// ---- 持久化 ----------------------------------------------------------------

/// 配置文件路径：$XDG_CONFIG_HOME/MarkSpace/config.json，未设置时用 ~/.config。
pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default();
    base.join("MarkSpace").join("config.json")
}

/// 加载配置：文件不存在或解析失败时回退到默认值。
pub fn load_config() -> AppConfig {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 保存配置（创建目录、美化输出；失败静默）。
pub fn save_config(config: &AppConfig) {
    let Ok(json) = serde_json::to_string_pretty(config) else {
        return;
    };
    let path = config_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, json);
}

// ---- 设置对话框 -------------------------------------------------------------

/// 设置分组：新增设置组时加变体 + ALL + label()。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsGroup {
    Font,
    // 未来：Theme, Behavior, …
}

impl SettingsGroup {
    pub const ALL: [SettingsGroup; 1] = [SettingsGroup::Font];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Font => "字体",
        }
    }
}

/// 设置对话框：左侧分组导航 + 右侧设置项表单。
///
/// 交互模型：`pending` 是控件修改的工作副本（即时预览），`snapshot` 是打开时
/// 的配置（取消时回滚）。确定 → 提交到 Workspace.config 并写盘。
pub struct SettingsDialog {
    workspace: WeakEntity<Workspace>,
    /// 最近一次提交的配置：打开时等于快照，确定后推进（取消/关闭时回滚到它）。
    committed: AppConfig,
    /// 控件修改目标（即时预览）。
    pending: AppConfig,
    /// 当前激活的分组。
    active_group: SettingsGroup,
    /// 系统字体列表缓存（后台线程枚举）。
    font_names: Vec<String>,
    /// 字号输入框（界面 / 编辑区）。
    ui_size_input: Entity<InputState>,
    mono_size_input: Entity<InputState>,
    /// cx.subscribe 返回的 Subscription 必须持有，否则订阅立即失效。
    _subscriptions: Vec<Subscription>,
    focus_handle: FocusHandle,
}

impl SettingsDialog {
    /// 创建对话框（在 `cx.new` 闭包里调用，cx 为 `&mut App`）。
    pub fn new(
        workspace: WeakEntity<Workspace>,
        config: AppConfig,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let ui_size_input = cx.new(|cx| {
            InputState::new(window, cx)
                .min(8.0)
                .max(72.0)
                .default_value(format!("{}", config.font.ui_font_size))
        });
        let mono_size_input = cx.new(|cx| {
            InputState::new(window, cx)
                .min(8.0)
                .max(72.0)
                .default_value(format!("{}", config.font.mono_font_size))
        });
        Self {
            workspace,
            committed: config.clone(),
            pending: config,
            active_group: SettingsGroup::Font,
            font_names: Vec::new(),
            ui_size_input,
            mono_size_input,
            _subscriptions: Vec::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// 建立订阅并开始后台枚举字体（在实体创建后调用）。
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 订阅字号输入框：数值变更 → 更新 pending 并即时预览
        self._subscriptions.push(cx.subscribe(
            &self.ui_size_input,
            |this, _, _: &InputEvent, cx| {
                let current = this.pending.font.ui_font_size;
                let value: f32 = this
                    .ui_size_input
                    .read(cx)
                    .value()
                    .parse()
                    .unwrap_or(current);
                this.pending.font.ui_font_size = value.clamp(8.0, 72.0);
                this.preview(cx);
                cx.notify();
            },
        ));
        self._subscriptions.push(cx.subscribe(
            &self.mono_size_input,
            |this, _, _: &InputEvent, cx| {
                let current = this.pending.font.mono_font_size;
                let value: f32 = this
                    .mono_size_input
                    .read(cx)
                    .value()
                    .parse()
                    .unwrap_or(current);
                this.pending.font.mono_font_size = value.clamp(8.0, 72.0);
                this.preview(cx);
                cx.notify();
            },
        ));

        // 字体枚举放后台线程（TextSystem 是 Send+Sync 的 Arc，可跨线程调用；
        // window.text_system() 返回借用，需用 App 级的 Arc clone）
        let text_system = cx.text_system().clone();
        let this = cx.entity().downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let names = cx
                .background_executor()
                .spawn(async move { text_system.all_font_names() })
                .await;
            _ = cx.update(|_, cx| {
                if let Some(entity) = this.upgrade() {
                    entity.update(cx, |dialog, cx| {
                        dialog.font_names = names;
                        cx.notify();
                    });
                }
            });
        })
        .detach();
    }

    /// 把 pending 配置应用到运行时（即时预览，不修改 Workspace.config）。
    fn preview(&self, cx: &mut App) {
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| ws.apply_pending(&self.pending, cx));
        }
    }

    /// 确定：提交 pending 到 Workspace 配置并写盘，快照推进到 pending。
    /// 注意：确认按钮关闭时会走 on_close，此时回滚到 pending（无变化）。
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.committed = self.pending.clone();
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| {
                ws.commit_config(self.pending.clone(), window, cx)
            });
        }
    }

    /// 回滚到最近一次提交（打开时的快照，或确定后的值）——取消/关闭路径共用。
    pub fn rollback(&mut self, cx: &mut Context<Self>) {
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| ws.apply_pending(&self.committed, cx));
        }
    }

    /// 恢复默认：pending 重置为出厂值并即时预览。
    fn restore_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = AppConfig::default();
        self.sync_size_inputs(window, cx);
        self.preview(cx);
        cx.notify();
    }

    /// 重置缩放系数（界面 + 编辑区），字号与字体不变。
    fn reset_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| ws.reset_zoom(window, cx));
        }
        cx.notify();
    }

    /// 把 pending 字号同步到输入框（恢复默认/预设按钮后）。
    fn sync_size_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ui = self.pending.font.ui_font_size;
        let mono = self.pending.font.mono_font_size;
        self.ui_size_input.update(cx, |state, cx| {
            state.set_value(format!("{ui}"), window, cx);
        });
        self.mono_size_input.update(cx, |state, cx| {
            state.set_value(format!("{mono}"), window, cx);
        });
    }

    fn set_ui_family(&mut self, family: String, cx: &mut Context<Self>) {
        self.pending.font.ui_font_family = family;
        self.preview(cx);
        cx.notify();
    }

    fn set_mono_family(&mut self, family: String, cx: &mut Context<Self>) {
        self.pending.font.mono_font_family = family;
        self.preview(cx);
        cx.notify();
    }

    // ---- 字体组表单 ----

    fn render_font_group(&mut self, _: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        // 两个下拉菜单闭包各自持有字体列表（move 捕获）
        let font_names_ui = self.font_names.clone();
        let font_names_mono = self.font_names.clone();
        let ui_family = self.pending.font.ui_font_family.clone();
        let mono_family = self.pending.font.mono_font_family.clone();
        let this = cx.entity().downgrade();

        // 界面字体下拉：菜单项来自后台枚举的字体列表
        let ui_font_button = Button::new("ui-font")
            .ghost()
            .w_full()
            .label(if ui_family == ".SystemUIFont" {
                "系统默认 (.SystemUIFont)".into()
            } else {
                ui_family.clone()
            })
            .dropdown_menu({
                let this = this.clone();
                move |menu, _, _| {
                    let mut menu = menu.min_w(px(280.)).max_h(px(320.)).scrollable(true);
                    menu = menu.item(
                        PopupMenuItem::new("系统默认 (.SystemUIFont)")
                            .checked(ui_family == ".SystemUIFont")
                            .on_click({
                                let this = this.clone();
                                move |_, _, cx| {
                                    if let Some(d) = this.upgrade() {
                                        d.update(cx, |d, cx| {
                                            d.set_ui_family(".SystemUIFont".into(), cx)
                                        });
                                    }
                                }
                            }),
                    );
                    for name in &font_names_ui {
                        let name = name.clone();
                        menu = menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(name == ui_family)
                                .on_click({
                                    let this = this.clone();
                                    move |_, _, cx| {
                                        if let Some(d) = this.upgrade() {
                                            d.update(cx, |d, cx| d.set_ui_family(name.clone(), cx));
                                        }
                                    }
                                }),
                        );
                    }
                    menu
                }
            });

        // 等宽字体下拉
        let mono_font_button = Button::new("mono-font")
            .ghost()
            .w_full()
            .label(if mono_family == ".SystemUIFont" {
                "系统默认 (.SystemUIFont)".into()
            } else {
                mono_family.clone()
            })
            .dropdown_menu({
                let this = this.clone();
                move |menu, _, _| {
                    let mut menu = menu.min_w(px(280.)).max_h(px(320.)).scrollable(true);
                    menu = menu.item(
                        PopupMenuItem::new("系统默认 (.SystemUIFont)")
                            .checked(mono_family == ".SystemUIFont")
                            .on_click({
                                let this = this.clone();
                                move |_, _, cx| {
                                    if let Some(d) = this.upgrade() {
                                        d.update(cx, |d, cx| {
                                            d.set_mono_family(".SystemUIFont".into(), cx)
                                        });
                                    }
                                }
                            }),
                    );
                    for name in &font_names_mono {
                        let name = name.clone();
                        menu = menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(name == mono_family)
                                .on_click({
                                    let this = this.clone();
                                    move |_, _, cx| {
                                        if let Some(d) = this.upgrade() {
                                            d.update(cx, |d, cx| {
                                                d.set_mono_family(name.clone(), cx)
                                            });
                                        }
                                    }
                                }),
                        );
                    }
                    menu
                }
            });

        // 缩放信息行 + 重置缩放
        let zoom = self
            .workspace
            .upgrade()
            .map(|ws| ws.read(cx).zoom_status(cx))
            .unwrap_or((1.0, 1.0));
        let reset_zoom = Button::new("reset-zoom")
            .ghost()
            .xsmall()
            .label("重置缩放")
            .on_click({
                let this = this.clone();
                move |_, window, cx| {
                    if let Some(d) = this.upgrade() {
                        d.update(cx, |d, cx| d.reset_zoom(window, cx));
                    }
                }
            });

        let ui_input = self.ui_size_input.clone();
        let mono_input = self.mono_size_input.clone();

        v_flex()
            .gap_5()
            .child(
                v_flex()
                    .gap_1()
                    .child(form_label(cx, "界面字体"))
                    .child(ui_font_button),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(form_label(cx, "编辑器等宽字体"))
                    .child(mono_font_button),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(form_label(cx, "界面基准字号"))
                    // 输入框加宽并随容器自适应，避免字体放大后数字/步进按钮被遮挡
                    .child(
                        NumberInput::new(&ui_input)
                            .w_full()
                            .max_w(px(240.))
                            .appearance(true),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(form_label(cx, "编辑区基准字号"))
                    .child(
                        NumberInput::new(&mono_input)
                            .w_full()
                            .max_w(px(240.))
                            .appearance(true),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .text_sm()
                            .child(format!(
                                "整体缩放 {:.0}% · 编辑区 {:.0}%（Ctrl+/-、Ctrl+滚轮调节）",
                                zoom.0 * 100.0,
                                zoom.1 * 100.0
                            )),
                    )
                    .child(reset_zoom),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("restore-defaults")
                        .ghost()
                        .xsmall()
                        .label("恢复默认")
                        .on_click({
                            let this = this.clone();
                            move |_, window, cx| {
                                if let Some(d) = this.upgrade() {
                                    d.update(cx, |d, cx| d.restore_defaults(window, cx));
                                }
                            }
                        }),
                ),
            )
    }
}

impl EventEmitter<InputEvent> for SettingsDialog {}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();

        // 左侧分组导航
        let nav_items = SettingsGroup::ALL
            .iter()
            .map(|group| {
                let group = *group;
                let selected = group == self.active_group;
                let this = this.clone();
                Button::new(format!("group-{:?}", group))
                    .ghost()
                    .w_full()
                    .label(group.label())
                    .selected(selected)
                    .on_click(move |_, _, cx| {
                        if let Some(d) = this.upgrade() {
                            d.update(cx, |d, cx| {
                                d.active_group = group;
                                cx.notify();
                            });
                        }
                    })
            })
            .collect::<Vec<_>>();

        let body = match self.active_group {
            SettingsGroup::Font => self.render_font_group(window, cx).into_any_element(),
        };

        h_flex()
            .size_full()
            .h(px(440.))
            .child(
                v_flex()
                    .w(px(140.))
                    .p_3()
                    .gap_1()
                    .bg(cx.theme().colors.muted)
                    .children(nav_items),
            )
            .child(div().w_px().h_full().bg(cx.theme().border))
            // 内容区可滚动：整体缩放放大字号后内容超出时不被遮挡
            .child(
                v_flex()
                    .id("settings-body")
                    .flex_1()
                    .p_4()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

/// 表单行的标题文字。
fn form_label(cx: &mut App, text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}
