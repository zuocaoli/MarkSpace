//! 目录树面板：后台扫描工作目录，只读浏览（展开/折叠、点击打开）。

use crate::editor_panel::EditorPanel;
use crate::model;
use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, WeakEntity, Window, hsla, img, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::dock::{BasePanel, Panel, PanelControl, PanelEvent};
use gpui_component::list::ListItem;
use gpui_component::tree::{TreeItem, TreeState, tree};
use gpui_component::{Sizable, h_flex, v_flex};
use std::path::{Path, PathBuf};

pub struct TreePanel {
    root: PathBuf,
    tree_state: Entity<TreeState>,
    items: Vec<TreeItem>,
    editor_panel: WeakEntity<EditorPanel>,
    focus_handle: FocusHandle,
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
                });
            }
        })
        .detach();
    }

    /// 切换工作根目录并重新扫描（先清空旧树，避免残留节点）。
    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.root = root;
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor_panel = self.editor_panel.clone();
        let this = cx.entity().downgrade();

        // 侧栏不做纸质：内容区用内置浅色板的侧栏背景（#fafafa 近白），
        // 与纸面米白的中央区域区分
        v_flex()
            .size_full()
            .bg(hsla(0.0, 0.0, 0.98, 1.0))
            .child(tree(
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
                    ListItem::new(ix)
                        .w_full()
                        .px_3()
                        .pl(px(14.) * entry.depth() as f32 + px(10.))
                        .child(h_flex().gap_2().child(icon).child(item.label.clone()))
                        .on_click({
                            let editor_panel = editor_panel.clone();
                            let this = this.clone();
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
            ))
    }
}
