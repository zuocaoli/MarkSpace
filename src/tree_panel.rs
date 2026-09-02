//! 目录树面板：后台扫描工作目录；浏览（展开/折叠、点击打开）+ 右键重命名
//! （支持对文件与目录改名，已打开的文档路径自动同步）。

use crate::editor_panel::{EditorPanel, push_notification};
use crate::model;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, WeakEntity, Window, hsla, img, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::dock::{BasePanel, Panel, PanelControl, PanelEvent};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::list::ListItem;
use gpui_component::menu::PopupMenuItem;
use gpui_component::notification::Notification;
use gpui_component::tree::{TreeItem, TreeState, tree};
use gpui_component::{Sizable, h_flex, v_flex};
use std::path::{Path, PathBuf};

pub struct TreePanel {
    root: PathBuf,
    tree_state: Entity<TreeState>,
    items: Vec<TreeItem>,
    editor_panel: WeakEntity<EditorPanel>,
    focus_handle: FocusHandle,
    /// 正在重命名的节点完整路径；Some 期间该项 label 渲染为输入框。
    renaming: Option<PathBuf>,
    /// 重命名输入框实体（懒创建，InputState::new 需要 window）。
    rename_input: Option<Entity<InputState>>,
    /// 重命名输入事件订阅（必须持有，否则订阅立即失效）。
    _subscriptions: Vec<gpui::Subscription>,
    /// 重扫描完成后要选中的节点（重命名后保持选中新节点）。
    select_after_rescan: Option<PathBuf>,
    /// Commit 在 subscribe 回调里没有 window，错误提示先暂存，render 帧内弹出。
    pending_rename_error: Option<String>,
}

impl TreePanel {
    pub fn new(
        root: PathBuf,
        editor_panel: WeakEntity<EditorPanel>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut panel = Self {
            root,
            tree_state: cx.new(|cx| TreeState::new(cx)),
            items: Vec::new(),
            editor_panel,
            focus_handle: cx.focus_handle(),
            renaming: None,
            rename_input: None,
            _subscriptions: Vec::new(),
            select_after_rescan: None,
            pending_rename_error: None,
        };
        panel.rescan(cx);
        panel
    }

    /// 重新扫描目录（后台线程执行 IO，避免卡 UI；
    /// TreeItem 含 Rc 不可跨线程，故用 Send 的中间表示扫描、主线程再转换）。
    /// 空工作区（尚未打开目录）不扫描。
    pub fn rescan(&mut self, cx: &mut Context<Self>) {
        if self.root.as_os_str().is_empty() {
            self.items.clear();
            self.tree_state
                .update(cx, |state, cx| state.set_items(Vec::new(), cx));
            return;
        }
        let root = self.root.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let node = cx
                .background_executor()
                .spawn(async move { model::scan_node(&root) })
                .await;
            let Some(node) = node else { return };
            let items = model::nodes_to_items(node.children);
            if let Some(entity) = this.upgrade() {
                entity.update(cx, |panel, cx| {
                    panel.items = items.clone();
                    panel.tree_state.update(cx, |state, cx| {
                        state.set_items(items, cx);
                    });
                    // 重命名后保持选中新节点
                    if let Some(target) = panel.select_after_rescan.take() {
                        panel.select_path(&target, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// 切换工作根目录并重新扫描（先清空旧树，避免残留节点）。
    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.root = root;
        // 切换目录时退出任何进行中的重命名
        self.renaming = None;
        self.items.clear();
        self.tree_state
            .update(cx, |state, cx| state.set_items(Vec::new(), cx));
        self.rescan(cx);
    }

    /// 选中指定路径的节点（自动展开祖先并滚动到可见）。
    pub fn select_path(&self, path: &Path, cx: &mut Context<Self>) {
        let id = path.to_string_lossy().into_owned();
        if let Some(item) = model::find_item(&self.items, &id) {
            self.tree_state.update(cx, |state, cx| {
                state.set_selected_item(Some(item), cx);
            });
        }
    }

    /// 开始重命名节点：预填当前名称、全选并聚焦输入框。
    /// 已在重命名会话中则忽略（防重复触发）。
    pub fn start_rename(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.is_some() {
            return;
        }
        if self.rename_input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx));
            // 回车提交；焦点离开（点击别处）也提交。commit 幂等（take 掉会话）。
            // 订阅回调的 this 参数即面板自身，无需外部 WeakEntity。
            let subscription = cx.subscribe(&input, {
                move |this: &mut TreePanel,
                      _: Entity<InputState>,
                      event: &InputEvent,
                      cx: &mut Context<TreePanel>| match event {
                    InputEvent::PressEnter { .. } => this.commit_rename(cx),
                    InputEvent::Blur => this.commit_rename(cx),
                    _ => {}
                }
            });
            self.rename_input = Some(input);
            self._subscriptions.push(subscription);
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(input) = &self.rename_input {
            input.update(cx, |s, cx| s.set_value(name, window, cx));
            input.update(cx, |s, cx| s.select_all(window, cx));
        }
        self.renaming = Some(path.to_path_buf());
        // 延迟一帧再聚焦输入框：本帧右键菜单正在收尾（恢复 previous focus），
        // 立即聚焦会被菜单还原焦点触发的 Blur → commit 立刻终结编辑会话
        let input = self.rename_input.clone();
        window.defer(cx, move |window, cx| {
            if let Some(input) = input {
                input.read(cx).focus_handle(cx).focus(window, cx);
            }
        });
        cx.notify();
    }

    /// 提交重命名（subscribe 回调里调用，无 window，错误提示经 pending 字段）。
    /// 空名/含路径分隔符/`.`与`..`→ 静默取消；目标已存在 → 报错并取消。
    pub fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some(old) = self.renaming.take() else {
            return;
        };
        let new_name = self
            .rename_input
            .as_ref()
            .map(|i| i.read(cx).value())
            .unwrap_or_default()
            .to_string();
        let new_name = new_name.trim().to_string();

        // 空名或非法名：静默取消，不弹通知
        if new_name.is_empty()
            || new_name == "."
            || new_name == ".."
            || new_name.contains('/')
            || new_name.contains('\\')
        {
            cx.notify();
            return;
        }
        let new_path = old
            .parent()
            .map(|p| p.join(&new_name))
            .unwrap_or_else(|| PathBuf::from(&new_name));
        // 名字没变：无事发生
        if new_path == old {
            cx.notify();
            return;
        }
        if new_path.exists() {
            self.pending_rename_error = Some(format!("已存在同名文件/目录：{new_name}"));
            cx.notify();
            return;
        }
        match std::fs::rename(&old, &new_path) {
            Ok(()) => {
                // 同步编辑器中已打开文档的路径（文件重命名或目录重命名都覆盖），
                // 再刷新树并选中新节点
                if let Some(ep) = self.editor_panel.upgrade() {
                    ep.update(cx, |p, cx| p.on_paths_renamed(&old, &new_path, cx));
                    ep.update(cx, |p, cx| p.notify_workspace(cx));
                }
                self.select_after_rescan = Some(new_path);
                self.rescan(cx);
            }
            Err(err) => {
                self.pending_rename_error = Some(format!("重命名失败：{err}"));
                cx.notify();
            }
        }
    }
}

impl EventEmitter<PanelEvent> for TreePanel {}

impl Focusable for TreePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for TreePanel {
    fn panel_name(&self) -> &'static str {
        "MarkdownWorkspaceTree"
    }
}

impl Panel for TreePanel {
    /// 隐藏 dock 面板组标题栏的 Zoom 控件（省略号菜单中的放大/还原项）。
    fn zoom_control(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "目录"
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![
            Button::new("refresh-tree")
                .ghost()
                .xsmall()
                .label("刷新")
                .on_click(cx.listener(|this, _, _, cx| this.rescan(cx))),
        ])
    }
}

impl Render for TreePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 迟到错误提示（subscribe 回调没有 window，无法即时弹通知）。
        // defer 到下一帧再弹：render 期间同步更新 Root 有重入风险。
        if let Some(msg) = self.pending_rename_error.take() {
            window.defer(cx, move |window, cx| {
                push_notification(window, cx, Notification::error(msg));
            });
        }

        let editor_panel = self.editor_panel.clone();
        let this = cx.entity().downgrade();
        // 右键菜单构建闭包与行渲染闭包各自持有 this 的克隆（闭包都是 move）
        let this_menu = this.clone();
        let renaming = self.renaming.clone();
        let rename_input = self.rename_input.clone();

        // 侧栏不做纸质：内容区用内置浅色板的侧栏背景（#fafafa 近白），
        // 与纸面米白的中央区域区分
        v_flex().size_full().bg(hsla(0.0, 0.0, 0.98, 1.0)).child(
            tree(
                &self.tree_state,
                move |ix, entry, _selected, _window, _cx| {
                    let item = entry.item();
                    // id 即完整路径
                    let path = PathBuf::from(item.id.to_string());
                    // 文件夹/文件图标用自定义彩色 PNG（assets/icons/，img 元素走
                    // AssetSource 的 Embedded 资源，彩色渲染）
                    let icon = if entry.is_folder() {
                        img("icons/files.png").size(px(14.)).into_any_element()
                    } else {
                        img("icons/file.png").size(px(14.)).into_any_element()
                    };
                    let is_folder = entry.is_folder();
                    // 正在重命名的项：label 换成单行输入框
                    let is_renaming = renaming.as_ref() == Some(&path);
                    let label: gpui::AnyElement = if is_renaming {
                        match &rename_input {
                            Some(input) => Input::new(input)
                                .h(px(20.))
                                .flex_grow(1.0)
                                .into_any_element(),
                            None => item.label.clone().into_any_element(),
                        }
                    } else {
                        item.label.clone().into_any_element()
                    };

                    ListItem::new(ix)
                        .w_full()
                        .px_3()
                        .pl(px(14.) * entry.depth() as f32 + px(10.))
                        .child(h_flex().w_full().gap_2().child(icon).child(label))
                        .on_click({
                            let editor_panel = editor_panel.clone();
                            let this = this.clone();
                            let path = path.clone();
                            move |_, window, cx| {
                                // 同步选中树节点
                                if let Some(panel) = this.upgrade() {
                                    panel.update(cx, |p, cx| p.select_path(&path, cx));
                                }
                                // 目录点击交给树组件展开/折叠，不触发打开
                                if !is_folder && let Some(ep) = editor_panel.upgrade() {
                                    // open 不再内部通知 Workspace（避免嵌套更新），
                                    // 这里在 Workspace 栈外，显式通知刷新
                                    ep.update(cx, |p, cx| p.open(path.clone(), window, cx));
                                    ep.update(cx, |p, cx| p.notify_workspace(cx));
                                }
                            }
                        })
                },
            )
            .context_menu(move |_ix, entry, menu, _window, _cx| {
                // 右键菜单：重命名当前节点（重命名进行中由 start_rename 幂等拦截）
                let path = PathBuf::from(entry.item().id.to_string());
                let this = this_menu.clone();
                menu.item(PopupMenuItem::new("重命名").on_click(move |_, window, cx| {
                    if let Some(panel) = this.upgrade() {
                        panel.update(cx, |p, cx| p.start_rename(&path, window, cx));
                    }
                }))
            }),
        )
    }
}
