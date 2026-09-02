//! 编辑器面板：多文档内部 TabBar + 编辑区，持有全部文档状态。
//!
//! Ctrl+滚轮的实现说明：滚轮缩放必须在 Capture 阶段拦截——gpui 的
//! `window.on_mouse_event` 只能在 paint 阶段调用，而 div 的 `on_scroll_wheel`
//! 只在 Bubble 阶段回调，此时 Input 组件（编辑器）内部已经滚动并
//! `stop_propagation()` 了。因此这里用一个自定义元素，在 paint 阶段注册
//! Capture 监听器（元素树中它最先 paint，监听器排在事件分发的最前）。

/// 由滚轮增量换算缩放因子（上滚放大、下滚缩小；行滚轮按行高估为像素）。
fn scroll_zoom_factor(delta: gpui::ScrollDelta) -> Option<f32> {
    let y = match delta {
        gpui::ScrollDelta::Pixels(p) => p.y.as_f32(),
        gpui::ScrollDelta::Lines(l) => l.y * 12.0,
    };
    if y > 0.0 {
        Some(1.05)
    } else if y < 0.0 {
        Some(1.0 / 1.05)
    } else {
        None
    }
}

/// 滚轮缩放回调：参数为缩放因子。
type ZoomCallback = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// 零尺寸拦截元素：paint 阶段注册 Capture 阶段滚轮监听，
/// Ctrl+滚轮 → 缩放编辑/预览区字号并 stop_propagation（阻止内容同时滚动）。
struct CaptureWheelZoom {
    on_zoom: ZoomCallback,
}

impl CaptureWheelZoom {
    fn new(on_zoom: ZoomCallback) -> Self {
        Self { on_zoom }
    }
}

impl Element for CaptureWheelZoom {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // 零尺寸，不影响布局
        (window.request_layout(Style::default(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<gpui::Pixels>,
        _: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<gpui::Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let on_zoom = self.on_zoom.clone();
        window.on_mouse_event(move |event: &gpui::ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || !event.modifiers.control {
                return;
            }
            if let Some(factor) = scroll_zoom_factor(event.delta) {
                on_zoom(factor, window, cx);
                cx.stop_propagation();
            }
        });
    }
}

impl IntoElement for CaptureWheelZoom {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

// 为什么不用 Dock 的动态 tab：公开 API 没有「激活已存在面板」的方法，
// 重复 add_panel 会重复插入 tab，因此这里用单面板 + 内部 TabBar 管理多文档。
// 每个文档的编辑器/预览都是独立 Entity，切换 Tab 不丢光标、滚动与撤销栈。

use crate::model::{self, Heading};
use crate::workspace::Workspace;
use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, DispatchPhase, Element, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, GlobalElementId, InspectorElementId, IntoElement, LayoutId, SharedString, Style,
    Subscription, Task, WeakEntity, Window, div, relative,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::dock::{BasePanel, Panel, PanelControl, PanelEvent};
use gpui_component::input::{Editor, EditorState, InputEvent, Position, TabSize};
use gpui_component::notification::Notification;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::text::{TextView, TextViewState};
use gpui_component::{ActiveTheme, Icon, IconName, Root, Sizable, v_flex};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// 单个已打开文档的全部状态。
struct DocState {
    path: PathBuf,
    editor: Entity<EditorState>,
    /// 预览实体（与编辑器同区域切换显示）；编辑时防抖更新其文本。
    preview: Entity<TextViewState>,
    outline: Vec<Heading>,
    /// 上次保存到磁盘的文本快照，用于 dirty 判断。
    saved_text: SharedString,
    dirty: bool,
    /// cx.subscribe 返回的 Subscription 必须持有，否则订阅立即失效。
    _subscriptions: Vec<Subscription>,
    /// 预览防抖任务；重新赋值即取消旧任务。
    _preview_task: Option<Task<()>>,
}

pub struct EditorPanel {
    docs: HashMap<PathBuf, DocState>,
    tab_order: Vec<PathBuf>,
    active: Option<PathBuf>,
    /// true = 中央区域显示预览（与编辑器公用同一块区域）。
    preview_mode: bool,
    /// 编辑/预览区字号缩放系数（Ctrl+滚轮 或 Ctrl+/- 整体缩放时同步）。
    content_zoom: f32,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
}

impl EditorPanel {
    pub fn new(workspace: WeakEntity<Workspace>, cx: &mut App) -> Self {
        Self {
            docs: std::collections::HashMap::new(),
            tab_order: Vec::new(),
            active: None,
            preview_mode: false,
            content_zoom: 1.0,
            workspace,
            focus_handle: cx.focus_handle(),
        }
    }

    /// 按系数缩放编辑/预览区字号（限制在 50%–300%）。
    pub fn zoom_content(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.content_zoom = (self.content_zoom * factor).clamp(0.5, 3.0);
        cx.notify();
    }

    /// 直接设置编辑/预览区缩放系数（启动加载配置/重置缩放时调用）。
    pub fn set_content_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.content_zoom = zoom.clamp(0.5, 3.0);
        cx.notify();
    }

    /// 当前编辑/预览区缩放系数。
    pub fn content_zoom(&self) -> f32 {
        self.content_zoom
    }

    /// 打开文档：已打开则切换到对应 Tab，否则创建 DocState 并后台读取内容。
    pub fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // 仅支持 Markdown 文件
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            push_notification(
                window,
                cx,
                Notification::info(format!("仅支持打开 Markdown 文件：{name}")),
            );
            return;
        }
        if self.docs.contains_key(&path) {
            self.active = Some(path);
            cx.notify();
            // 不再通知 Workspace：open 可能被 Workspace 栈内调用（open_file/
            // open_readme），内部通知会嵌套更新 panic；栈外调用方（树点击）在
            // open 后显式调 notify_workspace
            return;
        }

        // 创建编辑器与预览实体（先占位，文件内容后台读取）
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .line_number(true)
                .tab_size(TabSize {
                    tab_size: 2,
                    ..Default::default()
                })
                .searchable(true)
                .placeholder("读取中…")
            // 后续语法高亮开关：.language(Language::Markdown) + set_highlighter_factory
        });
        let preview = cx.new(|cx| TextViewState::markdown("", cx));

        // 订阅编辑事件：内容变化 → 更新 dirty/大纲，防抖刷新预览
        let path_key = path.clone();
        let subscription = cx.subscribe(&editor, {
            move |this: &mut EditorPanel,
                  _editor: Entity<EditorState>,
                  _event: &InputEvent,
                  cx: &mut Context<EditorPanel>| {
                this.on_doc_changed(&path_key, cx);
            }
        });

        self.docs.insert(
            path.clone(),
            DocState {
                path: path.clone(),
                editor,
                preview,
                outline: Vec::new(),
                saved_text: SharedString::default(),
                dirty: false,
                _subscriptions: vec![subscription],
                _preview_task: None,
            },
        );
        // 后台读取文件内容（gpui 0.2 的 Context 没有 window()，
        // 需要 set_value 时用 cx.update 在窗口上下文里执行）
        let load_path = path.clone();
        let read_path = load_path.clone();
        self.tab_order.push(path.clone());
        self.active = Some(path);
        cx.notify();
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let content = cx
                .background_executor()
                .spawn(async move { std::fs::read_to_string(&read_path) })
                .await;
            _ = cx.update(|window, cx| {
                let Some(entity) = this.upgrade() else { return };
                entity.update(cx, |panel, cx| {
                    let Some(doc) = panel.docs.get_mut(&load_path) else {
                        return;
                    };
                    match &content {
                        Ok(text) => {
                            doc.saved_text = text.clone().into();
                            doc.editor
                                .update(cx, |e, cx| e.set_value(text.clone(), window, cx));
                            doc.preview
                                .update(cx, |p, cx| p.set_text(text.as_str(), cx));
                            doc.outline = model::extract_headings(text);
                        }
                        Err(_) => {
                            doc.editor
                                .update(cx, |e, cx| e.set_value("(无法读取文件)", window, cx));
                        }
                    }
                    cx.notify();
                    panel.notify_others(cx);
                });
            });
        })
        .detach();
    }

    /// 关闭全部文档（切换工作目录时调用；未保存修改随之丢弃）。
    pub fn close_all(&mut self, cx: &mut Context<Self>) {
        self.docs.clear();
        self.tab_order.clear();
        self.active = None;
        self.preview_mode = false;
        cx.notify();
    }

    /// 存在未保存修改的文档数（切换工作目录前提示用）。
    pub fn dirty_count(&self) -> usize {
        self.docs.values().filter(|d| d.dirty).count()
    }

    /// 指定文档是否有未保存修改（关闭前判断用）。
    pub fn is_dirty(&self, path: &Path) -> bool {
        self.docs.get(path).map(|d| d.dirty).unwrap_or(false)
    }

    /// 关闭文档（调用方应已处理未保存确认）。
    ///
    /// 注意：本方法**不**通知 Workspace（调用方都是 Workspace 栈内路径，再
    /// notify 会触发嵌套更新 panic），由调用方自行 `notify_panels` 刷新。
    pub fn close_doc(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.docs.remove(path).is_none() {
            return;
        }
        self.tab_order.retain(|p| p != path);
        if self.active.as_deref() == Some(path) {
            // 关闭活动文档后，回退到列表第一个 Tab
            self.active = self.tab_order.first().cloned();
        }
        cx.notify();
    }

    /// 切换活动 Tab。
    pub fn set_active(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.docs.contains_key(path) {
            self.active = Some(path.to_path_buf());
            cx.notify();
            self.notify_others(cx);
        }
    }

    /// 保存活动文档到磁盘。
    pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active.clone() else {
            return;
        };
        let Some(doc) = self.docs.get_mut(&path) else {
            return;
        };
        let text = doc.editor.read(cx).value();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::write(&path, text.as_str()) {
            Ok(()) => {
                doc.saved_text = text;
                doc.dirty = false;
                push_notification(window, cx, Notification::success(format!("已保存 {name}")));
                cx.notify();
                // 不通知 Workspace：本方法由 Workspace::on_save 调用（其栈内），
                // 由调用方自行刷新，避免嵌套更新 panic
            }
            Err(err) => {
                push_notification(
                    window,
                    cx,
                    Notification::error(format!("保存失败 {name}: {err}")),
                );
            }
        }
    }

    /// 大纲跳转：把光标定位到指定行（内部自动滚动并聚焦编辑器）。
    pub fn jump_active(&mut self, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active.clone() else {
            return;
        };
        let Some(doc) = self.docs.get(&path) else {
            return;
        };
        let position = Position { line, character: 0 };
        doc.editor
            .update(cx, |e, cx| e.set_cursor_position(position, window, cx));
    }

    // ---- 读取接口（供大纲/状态栏面板使用）----

    pub fn active_outline(&self) -> Vec<Heading> {
        self.active_doc()
            .map(|d| d.outline.clone())
            .unwrap_or_default()
    }

    /// 状态栏信息：(相对显示路径, 是否未保存, 字符数)。
    pub fn status_text(&self, root: &Path, cx: &App) -> (String, bool, usize) {
        let Some(doc) = self.active_doc() else {
            return (String::new(), false, 0);
        };
        let display = doc
            .path
            .strip_prefix(root)
            .unwrap_or(&doc.path)
            .to_string_lossy()
            .into_owned();
        let chars = doc.editor.read(cx).value().chars().count();
        (display, doc.dirty, chars)
    }

    fn active_doc(&self) -> Option<&DocState> {
        self.active.as_ref().and_then(|p| self.docs.get(p))
    }

    /// 通知工作区刷新大纲/状态栏（供 Workspace 栈外的调用方在调用 open 等
    /// 方法后显式触发；Workspace 栈内调用方直接走自己的刷新）。
    pub fn notify_workspace(&self, cx: &mut App) {
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |w, cx| w.notify_panels(cx));
        }
    }

    fn active_doc_mut(&mut self) -> Option<&mut DocState> {
        self.active.as_ref().and_then(|p| self.docs.get_mut(p))
    }

    /// 切换 编辑/预览 视图（两者公用中央区域，切换 Tab 时状态保留）。
    pub fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_mode = !self.preview_mode;
        if self.preview_mode {
            // 进入预览时立即用编辑器当前内容刷新一次，避免等待防抖
            self.flush_preview(cx);
        }
        cx.notify();
    }

    /// 用编辑器当前内容同步刷新活动文档预览。
    fn flush_preview(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.active_doc_mut() else {
            return;
        };
        let text = doc.editor.read(cx).value();
        doc.preview
            .update(cx, |p, cx| p.set_text(text.as_str(), cx));
    }

    fn notify_others(&self, cx: &mut App) {
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |w, cx| w.notify_panels(cx));
        }
    }

    /// 编辑器内容变化：更新 dirty 与大纲缓存，重启预览防抖。
    fn on_doc_changed(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        let text = doc.editor.read(cx).value();
        doc.dirty = text != doc.saved_text;
        doc.outline = model::extract_headings(&text);

        // 300ms 防抖更新预览；重新赋值 Task 即取消旧任务
        let preview = doc.preview.clone();
        let path = path.to_path_buf();
        doc._preview_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let Some(entity) = this.upgrade() else { return };
            let text = entity.update(cx, |panel, cx| {
                panel.docs.get(&path).map(|d| d.editor.read(cx).value())
            });
            let Some(text) = text else { return };
            // 预览实体被 Task 持有克隆，文档可已关闭；此时更新无害（不再被渲染）
            preview.update(cx, |p, cx| p.set_text(text.as_str(), cx));
        }));

        cx.notify();
        self.notify_others(cx);
    }
}

impl EventEmitter<PanelEvent> for EditorPanel {}

impl Focusable for EditorPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for EditorPanel {
    fn panel_name(&self) -> &'static str {
        "MarkdownWorkspaceEditor"
    }
}

impl Panel for EditorPanel {
    /// 隐藏 dock 面板组标题栏的 Zoom 控件（省略号菜单中的放大/还原项）。
    fn zoom_control(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "编辑器"
    }

    fn tab_name(&self, _cx: &App) -> Option<SharedString> {
        Some(
            self.active
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "编辑器".to_string())
                .into(),
        )
    }

    /// 编辑器需要顶满面板，去掉内容内边距。
    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl Render for EditorPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_path = self.active.clone();
        let workspace = self.workspace.clone();
        let preview_mode = self.preview_mode;

        // 内容区 Ctrl+滚轮 → 字号缩放（Capture 拦截见 CaptureWheelZoom 注释）。
        // 作为 v_flex 第一个 child：最先 paint，监听器排在事件分发最前。
        // 经由 Workspace 中转，缩放系数同步写入配置并保存。
        let on_zoom = {
            let workspace = self.workspace.clone();
            Rc::new(move |factor: f32, window: &mut Window, cx: &mut App| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(cx, |ws, cx| ws.zoom_content(factor, window, cx));
                }
            })
        };

        // 编辑/预览 切换按钮（Tab 栏最右端）
        let preview_button = Button::new("toggle-preview")
            .ghost()
            .xsmall()
            .icon(if preview_mode {
                IconName::EyeOff
            } else {
                IconName::Eye
            })
            .label(if preview_mode { "编辑" } else { "预览" })
            .tooltip(if preview_mode {
                "切换回编辑视图"
            } else {
                "预览渲染效果"
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_preview(cx)));

        // 组装 Tab 栏
        let tabs = self
            .tab_order
            .iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let dirty = self.docs.get(path).map(|d| d.dirty).unwrap_or(false);
                let label = if dirty {
                    format!("{name} ●")
                } else {
                    name.clone()
                };
                let close_id = format!("close-tab-{}", path.display());
                // 注意：此版本 Tab::icon() 是「纯图标 tab」语义（有 icon 就不渲染
                // label），文件图标要走 prefix 与文件名并存
                Tab::new()
                    .label(label)
                    .prefix(Icon::new(IconName::FileText).size_4())
                    .suffix(
                        Button::new(close_id)
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .on_click({
                                let path = path.clone();
                                let workspace = workspace.clone();
                                move |_, window, cx| {
                                    if let Some(ws) = workspace.upgrade() {
                                        ws.update(cx, |w, cx| {
                                            w.request_close(path.clone(), window, cx)
                                        });
                                    }
                                }
                            }),
                    )
            })
            .collect::<Vec<_>>();
        let active_ix = self
            .tab_order
            .iter()
            .position(|p| Some(p) == active_path.as_ref())
            .unwrap_or(0);

        v_flex()
            .size_full()
            .child(CaptureWheelZoom::new(on_zoom))
            .child(
                TabBar::new("docs")
                    .underline()
                    .selected_index(active_ix)
                    .children(tabs)
                    .suffix(preview_button)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Some(path) = this.tab_order.get(*ix).cloned() {
                            this.set_active(&path, cx);
                        }
                    })),
            )
            .child(div().flex_1().min_h_0().child(match &active_path {
                Some(path) if self.docs.contains_key(path) => {
                    let doc = &self.docs[path];
                    if preview_mode {
                        // 预览与编辑器公用同一块区域，用按钮切换
                        let preview = doc.preview.clone();
                        let doc_path = doc.path.clone();
                        let workspace = self.workspace.clone();
                        div()
                            .id("preview")
                            .size_full()
                            .p_4()
                            .child(
                                TextView::new(&preview)
                                    .scrollable(true)
                                    .selectable(true)
                                    // 显式字体/字号：base 自绘的 TextView 不继承父容器
                                    // 字体（TextViewStyle 默认无 font_family），需在此指定。
                                    // theme.mono_font_size 已含 content_zoom，代码块经由
                                    // theme tokens（mono_md）自动跟随同一字号
                                    .font_family(cx.theme().mono_font_family.clone())
                                    .text_size(cx.theme().mono_font_size)
                                    // 链接点击：相对路径打开对应文件，http(s) 交系统浏览器
                                    .on_link_click(move |url, _event, window, cx| {
                                        handle_preview_link(&doc_path, url, &workspace, window, cx);
                                    }),
                            )
                            .into_any_element()
                    } else {
                        // 编辑器包在等宽字体容器里
                        div()
                            .id("source")
                            .size_full()
                            .child(
                                Editor::new(&doc.editor)
                                    .h(relative(1.))
                                    .p_0()
                                    .border_0()
                                    // theme.mono_font_size 已含 content_zoom，无需再乘
                                    .text_size(cx.theme().mono_font_size),
                            )
                            .into_any_element()
                    }
                }
                _ => empty_state(cx).into_any_element(),
            }))
    }
}

fn empty_state(cx: &mut Context<EditorPanel>) -> gpui::Div {
    div()
        .flex()
        .size_full()
        .items_center()
        .justify_center()
        .text_color(cx.theme().muted_foreground)
        .child("在左侧目录树中打开 Markdown 文件")
}

/// 预览链接点击：相对路径 → 打开对应文件；http(s) → 系统浏览器；锚点忽略。
fn handle_preview_link(
    doc_path: &Path,
    url: &SharedString,
    workspace: &WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let url_str = url.as_ref();
    if url_str.starts_with('#') {
        return; // 锚点暂不处理
    }
    if url_str.starts_with("http://") || url_str.starts_with("https://") {
        cx.open_url(url_str);
        return;
    }
    // 相对当前文档所在目录解析
    let target = if let Some(parent) = doc_path.parent() {
        parent.join(url_str)
    } else {
        PathBuf::from(url_str)
    };
    if !target.is_file() {
        return;
    }
    if let Some(ws) = workspace.upgrade() {
        ws.update(cx, |ws, cx| ws.open_file(target, window, cx));
    }
}

/// 推送一条通知（依赖窗口第一层的 Root 已挂载通知层）。
pub(crate) fn push_notification(window: &mut Window, cx: &mut App, notification: Notification) {
    Root::update(window, cx, |root, window, cx| {
        root.push_notification(notification, window, cx);
    });
}
