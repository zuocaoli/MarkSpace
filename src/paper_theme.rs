//! 纸质感主题：暖米白「书籍纸」色板，直接覆盖默认浅色主题。
//!
//! 原理：组件颜色每帧实时读全局 `Theme` 的 `ThemeColor` 字段（`cx.theme()` 经
//! Deref 到 colors），改字段 + `Theme::sync_base` + `window.refresh` 即全局生效。
//! 注意 `ThemeColor::default()` 是全透明黑，因此这里改的是 gpui_component::init
//! 已装好的内置浅色板（以 `*ThemeColor::light()` 为基底），不是从零构造。

use gpui::{App, Window, hsla};
use gpui_component::{Theme, ThemeColor, ThemeMode, ThemeTokens};

/// 应用纸质感色板（在 gpui_component::init 之后、窗口可见之前调用一次）。
pub fn apply(window: &mut Window, cx: &mut App) {
    // 以内置浅色板为基底（解引用 Arc 复制），避免 derive Default 的全透明字段
    let base = *ThemeColor::light();
    let mut colors = base;
    // —— 纸面基础 ——
    colors.background = hsla(42.0 / 360.0, 0.45, 0.93, 1.0); // 暖米白 #F6F1E3
    colors.foreground = hsla(40.0 / 360.0, 0.14, 0.22, 1.0); // 深暖墨 #3F3A30
    colors.border = hsla(44.0 / 360.0, 0.33, 0.84, 1.0); // 暖灰 #E3DCC8
    colors.muted = hsla(43.0 / 360.0, 0.38, 0.88, 1.0); // 悬停底 #ECE6D4
    colors.muted_foreground = hsla(43.0 / 360.0, 0.11, 0.50, 1.0); // #8B8472
    colors.input = hsla(42.0 / 360.0, 0.50, 0.96, 1.0); // 输入面 #FBF8EF
    colors.selection = hsla(44.0 / 360.0, 0.30, 0.79, 1.0); // #D9D0B8
    // —— 面板/结构（略深于纸面的米黄，衬托纸面）——
    // 注意：sidebar 系列保持内置浅色板默认（#fafafa 近白），两侧栏不做纸质；
    // 侧栏面板内容区在 TreePanel/OutlinePanel 的 render 里显式用同色背景
    colors.status_bar = hsla(42.0 / 360.0, 0.36, 0.90, 1.0);
    colors.status_bar_border = colors.border;
    colors.title_bar = hsla(42.0 / 360.0, 0.36, 0.90, 1.0);
    colors.title_bar_border = colors.border;
    colors.tab_bar = hsla(42.0 / 360.0, 0.36, 0.90, 1.0);
    colors.tab = colors.tab_bar;
    colors.tab_active = colors.background; // 选中 Tab 融入纸面
    colors.tab_foreground = colors.muted_foreground;
    colors.tab_active_foreground = colors.foreground;
    colors.popover = hsla(42.0 / 360.0, 0.42, 0.94, 1.0); // 对话框面 #F8F4E8
    colors.popover_foreground = colors.foreground;
    colors.scrollbar = hsla(43.0 / 360.0, 0.15, 0.60, 0.20);
    colors.scrollbar_thumb = hsla(43.0 / 360.0, 0.21, 0.73, 1.0); // #C8BFAB
    colors.scrollbar_thumb_hover = hsla(43.0 / 360.0, 0.19, 0.66, 1.0); // #B8AE97
    // —— 强调色：与蓝色文件夹图标协调的低饱和蓝 ——
    colors.accent = hsla(210.0 / 360.0, 0.58, 0.44, 1.0); // #2F6FB3
    colors.accent_foreground = hsla(0.0, 0.0, 1.0, 1.0);
    colors.link = colors.accent;
    colors.link_hover = hsla(210.0 / 360.0, 0.60, 0.35, 1.0);
    // —— 其余字段（按钮/危险/成功等）保持内置浅色板 ——

    // 安装：写入全局 Theme 的 colors 并同步 Base 层（滚动条等）
    let theme = Theme::global_mut(cx);
    theme.colors = colors;
    // 重建 tokens 快照：dock 面板主体等组件读 `theme.tokens.background`（旧快照
    // 不会跟随字段直改），不重建的话预览区仍显示旧的白色背景
    theme.tokens = ThemeTokens::from(&theme.colors);
    theme.mode = ThemeMode::Light;
    Theme::sync_base(cx);
    window.refresh();
}
