//! Workspace 根视图：Dock 三栏布局组装、全局动作（保存/切换面板）、状态栏、通知层。
//!
//! 布局：左 dock = 目录树；中央 = 编辑器面板（内部多文档 TabBar）；
//! 右 dock = 大纲面板。大纲为静态面板，切换文档时读取活动文档内容。

use crate::editor_panel::{EditorPanel, push_notification};
use crate::outline_panel::OutlinePanel;
use crate::settings::{self, AppConfig, SettingsDialog};
use crate::tree_panel::TreePanel;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, KeyBinding, PathPromptOptions, Pixels, WeakEntity, Window, actions, div,
    px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::dialog::{DialogAction, DialogButtonProps, DialogClose, DialogFooter};
use gpui_component::dock::{DockArea, DockLayout, DockPlacement, DockSkin, panel_handle};
use gpui_component::notification::Notification;
use gpui_component::status_bar::StatusBar;
use gpui_component::{ActiveTheme, IconName, Root, Sizable, Theme, WindowExt};
use std::path::PathBuf;
use std::rc::Rc;

// 全局动作定义
actions!(markdown_workspace, [Save, ToggleRightDock, ZoomIn, ZoomOut]);

const CONTEXT: &str = "MarkdownWorkspace";

/// 注册全局按键（必须在 main 中 gpui_component::init 之后调用）。
pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-s", Save, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-v", ToggleRightDock, Some(CONTEXT)),
        // 整体字体缩放（"ctrl-+" 即 Shift+=）
        KeyBinding::new("ctrl-=", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("ctrl-+", ZoomIn, Some(CONTEXT)),
        KeyBinding::new("ctrl--", ZoomOut, Some(CONTEXT)),
    ]);
}

pub struct Workspace {
    root: PathBuf,
    dock_area: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    /// 面板句柄需持有以保证实体存活（仅启动时使用）。
    #[allow(dead_code)]
    tree_panel: Entity<TreePanel>,
    editor_panel: Entity<EditorPanel>,
    outline_panel: Entity<OutlinePanel>,
    /// 应用配置（设置对话框确定后更新；字体/字号/缩放）。
    config: AppConfig,
    /// 整体界面缩放系数（Ctrl+/-），范围 50%–300%。
    ui_zoom: f32,
    /// 启动时的窗口 rem 基准，缩放时按系数重算。
    base_rem: Pixels,
    /// 设置对话框实体（每次打开时重建以刷新配置快照；经 Root 对话框层管理）。
    settings_dialog: Entity<SettingsDialog>,
}

impl Workspace {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 纸质感主题（暖米白书籍纸）覆盖默认浅色
        crate::paper_theme::apply(window, cx);

        let this = cx.entity();

        let editor_panel = cx.new(|cx| EditorPanel::new(this.downgrade(), cx));
        let outline_panel = cx.new(|cx| OutlinePanel::new(editor_panel.downgrade(), cx));
        let tree_panel = cx.new(|cx| TreePanel::new(root.clone(), editor_panel.downgrade(), cx));

        // 组装 Dock 布局（版本号用于失效旧布局缓存；移除预览面板后升到 2）
        let (dock_area, skin) = DockSkin::dock_area("markdown-workspace", Some(2), window, cx);
        dock_area.update(cx, |area, cx| {
            // 中央：编辑器
            area.set_center(
                DockLayout::tabs().panel_view(panel_handle(editor_panel.clone()), cx),
                window,
                cx,
            );
            // 左侧：目录树
            area.set_dock(
                DockPlacement::Left,
                DockLayout::tabs().panel_view(panel_handle(tree_panel.clone()), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Left, px(260.), window, cx);
            area.set_dock_collapsible(DockPlacement::Left, true, window, cx);
            // 右侧：大纲
            area.set_dock(
                DockPlacement::Right,
                DockLayout::tabs().panel_view(panel_handle(outline_panel.clone()), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Right, px(300.), window, cx);
            area.set_dock_collapsible(DockPlacement::Right, true, window, cx);
        });
        skin.set_toggle_button_visible(true, cx);

        // 加载持久化配置；首次运行或旧版占位（".SystemUIFont"）统一替换为默认字体
        let mut config = settings::load_config();
        for family in [
            &mut config.font.ui_font_family,
            &mut config.font.mono_font_family,
        ] {
            if family == ".SystemUIFont" {
                *family = settings::FontSettings::default().ui_font_family.clone();
            }
        }
        let ui_zoom = config.zoom.ui_zoom.clamp(0.5, 3.0);
        let content_zoom = config.zoom.content_zoom.clamp(0.5, 3.0);

        let settings_dialog =
            cx.new(|cx| SettingsDialog::new(this.downgrade(), config.clone(), window, cx));
        settings_dialog.update(cx, |dialog, cx| dialog.start(window, cx));

        let mut workspace = Self {
            root,
            dock_area,
            _skin: skin,
            tree_panel,
            editor_panel,
            outline_panel,
            config,
            ui_zoom,
            base_rem: window.rem_size(),
            settings_dialog,
        };
        workspace
            .editor_panel
            .update(cx, |ep, cx| ep.set_content_zoom(content_zoom, cx));
        // 启动时自动打开 README.md
        workspace.open_readme(window, cx);
        // 应用配置（字体/字号/缩放）
        workspace.apply_settings(window, cx);
        workspace
    }

    /// 打开设置对话框（状态栏「设置」按钮入口）。
    ///
    /// 用命令式 open_dialog：对话框由 Root 的对话框层管理，Esc/遮罩/按钮关闭后
    /// 自动从层移除，回调只负责提交/回滚配置。
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.config.clone();
        let this = cx.entity().downgrade();
        let dialog = cx.new(|cx| SettingsDialog::new(this, config, window, cx));
        dialog.update(cx, |dialog, cx| dialog.start(window, cx));
        let settings = dialog.clone();
        self.settings_dialog = dialog;
        window.open_dialog(cx, move |dialog, _, _cx| {
            // builder 是 Fn（每帧重跑），每个嵌套闭包各自持有 clone
            let settings_content = settings.clone();
            let settings_ok = settings.clone();
            let settings_cancel = settings.clone();
            let settings_close = settings.clone();
            dialog
                .w(px(640.))
                .title("设置")
                .content(move |content, _, _| content.child(settings_content.clone()))
                // 普通 Dialog 的 button_props 不自动渲染按钮，必须用 footer 显式提供
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .child(Button::new("settings-cancel").label("取消").ghost()),
                        )
                        .child(
                            DialogAction::new()
                                .child(Button::new("settings-ok").label("确定").primary()),
                        ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .on_ok(move |_, window, cx| {
                            // 确定：提交 pending 并写盘；对话框由层管理自动关闭
                            settings_ok.update(cx, |d, cx| d.confirm(window, cx));
                            true
                        })
                        .on_cancel(move |_, _, cx| {
                            // 取消按钮/遮罩：回滚到最近提交
                            settings_cancel.update(cx, |d, cx| d.rollback(cx));
                            true
                        }),
                )
                .on_close(move |_, _, cx| {
                    // Esc/关闭按钮：回滚到最近提交（确定后回滚无变化）
                    settings_close.update(cx, |d, cx| d.rollback(cx));
                })
        });
        cx.notify();
    }

    /// 把当前配置 + 缩放系数应用到全局主题/窗口。
    /// 组件字号/字体都来自全局主题，重绘后自动跟随。
    fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content_zoom = self.editor_panel.read(cx).content_zoom();
        self.apply_theme(cx, content_zoom);
        window.set_rem_size(self.base_rem * self.ui_zoom);
        cx.notify();
    }

    /// 把当前配置 + 缩放系数写入全局主题。
    /// `mono_font_size` 含 content_zoom（编辑区生效字号），经 sync_base 同步到
    /// 主题 tokens 后，预览的代码块字体大小会自动跟随（它读 tokens 而非元素样式）。
    fn apply_theme(&mut self, cx: &mut Context<Self>, content_zoom: f32) {
        let theme = Theme::global_mut(cx);
        theme.font_family = self.config.font.ui_font_family.clone().into();
        theme.mono_font_family = self.config.font.mono_font_family.clone().into();
        theme.font_size = px(self.config.font.ui_font_size) * self.ui_zoom;
        theme.mono_font_size = px(self.config.font.mono_font_size) * self.ui_zoom * content_zoom;
        Theme::sync_base(cx);
    }

    /// 应用对话框的临时配置（即时预览；传入快照即回滚，不修改 self.config）。
    /// 不涉及 rem 基准（ui_zoom 不变），故无需 window。
    pub fn apply_pending(&mut self, pending: &AppConfig, cx: &mut Context<Self>) {
        let saved = self.config.clone();
        self.config = pending.clone();
        let content_zoom = self.editor_panel.read(cx).content_zoom();
        self.apply_theme(cx, content_zoom);
        self.config = saved;
        cx.notify();
    }

    /// 设置对话框确定：提交配置并写盘。
    pub fn commit_config(
        &mut self,
        config: AppConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.config = config;
        settings::save_config(&self.config);
        self.apply_settings(window, cx);
    }

    /// 重置缩放系数（界面 + 编辑区）并保存（设置对话框「重置缩放」）。
    pub fn reset_zoom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ui_zoom = 1.0;
        self.config.zoom.ui_zoom = 1.0;
        self.config.zoom.content_zoom = 1.0;
        self.editor_panel
            .update(cx, |ep, cx| ep.set_content_zoom(1.0, cx));
        settings::save_config(&self.config);
        self.apply_settings(window, cx);
    }

    /// 编辑/预览区局部缩放（Ctrl+滚轮）：更新系数并保存，同时重算主题等宽字号
    /// （含 content_zoom），使预览代码块字体大小同步跟随。
    pub fn zoom_content(&mut self, factor: f32, _: &mut Window, cx: &mut Context<Self>) {
        self.editor_panel
            .update(cx, |ep, cx| ep.zoom_content(factor, cx));
        self.config.zoom.content_zoom = self.editor_panel.read(cx).content_zoom();
        settings::save_config(&self.config);
        let content_zoom = self.config.zoom.content_zoom;
        self.apply_theme(cx, content_zoom);
        cx.notify();
    }

    /// 当前缩放状态：(整体缩放系数, 编辑区缩放系数)，供设置对话框显示。
    pub fn zoom_status(&self, cx: &App) -> (f32, f32) {
        (self.ui_zoom, self.editor_panel.read(cx).content_zoom())
    }

    /// 打开指定文件（预览链接跳转等外部入口；已在 Workspace 栈内，负责刷新）。
    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_panel
            .update(cx, |ep, cx| ep.open(path, window, cx));
        self.notify_panels(cx);
    }

    /// 打开工作目录下的 README.md（启动或切换目录后调用；空工作区跳过）。
    fn open_readme(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.root.as_os_str().is_empty() {
            return;
        }
        let readme = self.root.join("README.md");
        if readme.is_file() {
            self.editor_panel
                .update(cx, |ep, cx| ep.open(readme, window, cx));
        }
    }

    /// 弹出系统目录选择器，选定后切换工作目录。
    fn on_open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择工作目录".into()),
        });
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let Ok(result) = rx.await else { return };
            match result {
                Ok(Some(paths)) => {
                    if let Some(path) = paths.into_iter().next() {
                        _ = cx.update(|window, cx| {
                            if let Some(entity) = this.upgrade() {
                                entity.update(cx, |ws, cx| ws.switch_root(path, window, cx));
                            }
                        });
                    }
                }
                // 用户取消
                Ok(None) => {}
                // 系统没有文件选择器后端（如 xdg-desktop-portal 缺失）
                Err(err) => {
                    _ = cx.update(|window, cx| {
                        if let Some(entity) = this.upgrade() {
                            entity.update(cx, |_, cx| {
                                push_notification(
                                    window,
                                    cx,
                                    Notification::error(format!("无法打开文件选择器：{err}")),
                                );
                            });
                        }
                    });
                }
            }
        })
        .detach();
    }

    /// 切换工作目录：关闭全部文档、重建目录树、打开新根目录下的 README。
    fn switch_root(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !path.is_dir() {
            push_notification(
                window,
                cx,
                Notification::error(format!("不是有效目录：{}", path.display())),
            );
            return;
        }
        let dirty = self.editor_panel.read(cx).dirty_count();
        self.editor_panel.update(cx, |ep, cx| ep.close_all(cx));
        self.tree_panel
            .update(cx, |tp, cx| tp.set_root(path.clone(), cx));
        self.open_readme(window, cx);
        let mut msg = format!("已切换到 {}", path.display());
        if dirty > 0 {
            msg.push_str(&format!("（丢弃 {dirty} 个未保存文档）"));
        }
        push_notification(window, cx, Notification::info(msg));
        cx.notify();
    }

    /// 编辑器面板状态变化时，统一刷新大纲/状态栏。
    pub fn notify_panels(&mut self, cx: &mut Context<Self>) {
        self.outline_panel.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    /// 请求关闭文档（编辑器面板 Tab 的关闭按钮入口）。
    ///
    /// 弹「放弃未保存的修改？」确认框，经 Root 对话框层管理（与设置对话框一致，
    /// 避免非受控渲染导致的关闭异常）。
    pub fn request_close(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // 没有未保存修改时直接关闭，不弹确认框
        if !self.editor_panel.read(cx).is_dirty(&path) {
            self.editor_panel
                .update(cx, |ep, cx| ep.close_doc(&path, cx));
            self.notify_panels(cx);
            return;
        }
        let ws = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _cx| {
            // builder 是 Fn（每帧重跑），嵌套闭包需各自持有 clone
            let ws = ws.clone();
            let path = path.clone();
            dialog
                .title("放弃未保存的修改？")
                // 普通 Dialog 的 button_props 不自动渲染按钮，必须用 footer 显式提供
                // （DialogClose 派发 Cancel action、DialogAction 派发 Confirm action）
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new()
                                .child(Button::new("confirm-close-cancel").label("取消").ghost()),
                        )
                        .child(
                            DialogAction::new()
                                .child(Button::new("confirm-close-ok").label("放弃").primary()),
                        ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .show_cancel(true)
                        .on_ok(move |_, _window, cx| {
                            if let Some(ws) = ws.upgrade() {
                                ws.update(cx, |w, cx| {
                                    w.editor_panel.update(cx, |ep, cx| ep.close_doc(&path, cx));
                                    w.notify_panels(cx);
                                });
                            }
                            true
                        })
                        .on_cancel(move |_, _, _| true),
                )
        });
        cx.notify();
    }

    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        self.editor_panel
            .update(cx, |ep, cx| ep.save_active(window, cx));
        // save_active 不再内部通知 Workspace（避免嵌套更新），这里统一刷新
        self.notify_panels(cx);
    }

    fn on_toggle_right_dock(
        &mut self,
        _: &ToggleRightDock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock_area.update(cx, |area, cx| {
            area.toggle_dock(DockPlacement::Right, window, cx)
        });
    }

    /// 整体界面缩放（Ctrl+/-）：更新系数、写入配置并保存，再统一应用。
    fn zoom_ui(&mut self, factor: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.ui_zoom = (self.ui_zoom * factor).clamp(0.5, 3.0);
        self.config.zoom.ui_zoom = self.ui_zoom;
        settings::save_config(&self.config);
        self.apply_settings(window, cx);
    }

    fn on_zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_ui(1.1, window, cx);
    }

    fn on_zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_ui(1.0 / 1.1, window, cx);
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dock_area = self.dock_area.clone();
        let (display_path, dirty, chars) = self.editor_panel.read(cx).status_text(&self.root, cx);

        let status_right = if display_path.is_empty() {
            "未打开文档".to_string()
        } else {
            format!(
                "{display_path}{} · {chars} 字符",
                if dirty { " · ● 未保存" } else { "" }
            )
        };

        // 状态栏：设置 + 打开目录 + 左右 dock 切换按钮 + 当前文档信息
        let settings_button = Button::new("settings")
            .ghost()
            .small()
            .icon(IconName::Settings)
            .tooltip("设置")
            .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)));
        let open_folder = Button::new("open-folder")
            .ghost()
            .small()
            .icon(IconName::FolderOpen)
            .tooltip("打开目录")
            .on_click(cx.listener(|this, _, window, cx| this.on_open_folder(window, cx)));
        let left_toggle = Button::new("toggle-left")
            .ghost()
            .small()
            .icon(IconName::PanelLeft)
            .on_click({
                let area = self.dock_area.clone();
                move |_, window, cx| {
                    area.update(cx, |a, cx| a.toggle_dock(DockPlacement::Left, window, cx));
                }
            });
        let right_toggle = Button::new("toggle-right")
            .ghost()
            .small()
            .icon(IconName::PanelRight)
            .on_click({
                let area = self.dock_area.clone();
                move |_, window, cx| {
                    area.update(cx, |a, cx| a.toggle_dock(DockPlacement::Right, window, cx));
                }
            });

        div()
            .id("markdown-workspace")
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_toggle_right_dock))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().flex_1().min_h_0().child(dock_area))
            .child(
                // 状态栏加高到 1.5 倍（原约 28px → 42px），配合 small 按钮放大图标
                StatusBar::new()
                    .h(px(42.))
                    .left(settings_button)
                    .left(open_folder)
                    .left(left_toggle)
                    .left(right_toggle)
                    .right(status_right),
            )
            // 对话框层（open_dialog 打开的对话框在此渲染，必须手动挂载）
            .children(Root::render_dialog_layer(window, cx))
            // 通知 toast 层（依赖 Root，需手动挂载）
            .children(Root::render_notification_layer(window, cx))
    }
}
