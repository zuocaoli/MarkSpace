//! Markdown Workspace — 基于 GPUI 的原生 Markdown 工作台。
//!
//! 用法：`cargo run -- [工作目录]`；缺省为空工作区（不打开目录，点击状态栏
//! 「打开目录」选择）。

mod editor_panel;
mod model;
mod outline_panel;
mod paper_theme;
mod settings;
mod tree_panel;
mod workspace;

use gpui::*;
use gpui_component::Root;
use std::borrow::Cow;
use std::path::PathBuf;

/// 应用资源：优先自己的 assets/（编译时嵌入），缺失时回退到组件图标库。
/// 这样既能用自定义图标（目录树文件夹/文件 PNG），又不影响
/// gpui_component 内置 IconName 图标的加载。
struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "icons/files.png" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/files.png"
            )))),
            "icons/file.png" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/file.png"
            )))),
            _ => gpui_component_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_component_assets::Assets.list(path)
    }
}

/// 加载软件自带字体（编译时嵌入二进制，不依赖系统安装的字体）。
/// 必须在 `gpui_component::init` 之后、窗口创建之前调用。
fn install_bundled_fonts(cx: &mut App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/YaHei-Consolas-Hybrid.ttf")),
        Cow::Borrowed(include_bytes!(
            "../assets/fonts/YaHei-Consolas-Hybrid-Bold.ttf"
        )),
    ];
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        eprintln!("加载内置字体失败: {err}");
    }
}

fn main() {
    // 解析命令行参数：第一个参数为工作目录；缺省为空 PathBuf（不代表任何目录）
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_default();
    if !root.as_os_str().is_empty() && !root.is_dir() {
        eprintln!("目录不存在：{}", root.display());
        std::process::exit(1);
    }

    gpui_platform::application()
        .with_assets(AppAssets) // 自定义资源 + 组件图标库
        .run(move |cx| {
            gpui_component::init(cx); // 必须最先调用
            install_bundled_fonts(cx); // 加载软件自带字体
            workspace::init(cx); // 全局按键绑定
            let root = root.clone();
            cx.spawn(async move |cx| {
                cx.open_window(WindowOptions::default(), move |window, cx| {
                    let view = cx.new(|cx| workspace::Workspace::new(root, window, cx));
                    // 窗口第一层视图必须是 Root —— 对话框/抽屉/通知都要靠它
                    cx.new(|cx| Root::new(view, window, cx))
                })
                .expect("Failed to open window");
            })
            .detach();
        });
}
