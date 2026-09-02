//! 大纲面板：列出活动文档的标题，点击跳转到编辑器对应位置。

use crate::editor_panel::EditorPanel;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, WeakEntity, Window, div, hsla, px,
};
use gpui_component::dock::{BasePanel, Panel, PanelControl, PanelEvent};
use gpui_component::{ActiveTheme, v_flex};

pub struct OutlinePanel {
    editor_panel: WeakEntity<EditorPanel>,
    focus_handle: FocusHandle,
}

impl OutlinePanel {
    pub fn new(editor_panel: WeakEntity<EditorPanel>, cx: &mut App) -> Self {
        Self {
            editor_panel,
            focus_handle: cx.focus_handle(),
        }
    }
}

impl EventEmitter<PanelEvent> for OutlinePanel {}

impl Focusable for OutlinePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for OutlinePanel {
    fn panel_name(&self) -> &'static str {
        "MarkdownWorkspaceOutline"
    }
}

impl Panel for OutlinePanel {
    /// 隐藏 dock 面板组标题栏的 Zoom 控件（省略号菜单中的放大/还原项）。
    fn zoom_control(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        "大纲"
    }
}

impl Render for OutlinePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let headings = self
            .editor_panel
            .upgrade()
            .map(|ep| ep.read(cx).active_outline())
            .unwrap_or_default();

        // 侧栏不做纸质：内容区用内置浅色板的侧栏背景（#fafafa 近白），
        // 与纸面米白的中央区域区分
        let sidebar_bg = hsla(0.0, 0.0, 0.98, 1.0);

        if headings.is_empty() {
            return v_flex()
                .size_full()
                .bg(sidebar_bg)
                .p_4()
                .text_color(cx.theme().muted_foreground)
                .child("当前文档没有标题")
                .into_any_element();
        }

        let editor_panel = self.editor_panel.clone();
        v_flex()
            .size_full()
            .id("outline")
            .bg(sidebar_bg)
            .overflow_y_scroll()
            .p_2()
            .children(headings.into_iter().map(|heading| {
                let line = heading.line;
                let editor_panel = editor_panel.clone();
                div()
                    .id(("outline-heading", line))
                    .px_3()
                    .py_1()
                    .pl(px(heading.level as f32 * 10. + 8.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().muted))
                    .child(heading.text)
                    .on_click(move |_, window, cx| {
                        editor_panel
                            .update(cx, |ep, cx| ep.jump_active(line, window, cx))
                            .ok();
                    })
            }))
            .into_any_element()
    }
}
