//! Windows 资源嵌入：把应用图标（assets/app.ico）与版本信息编入 .exe，
//! 使资源管理器/任务栏/Alt-Tab 显示 MarkSpace 图标。
//! 仅在 Windows 目标编译时生效，依赖见 Cargo.toml 的 cfg(windows) build-deps。

fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("FileDescription", "MarkSpace");
        res.set("ProductName", "MarkSpace");
        res.set("LegalCopyright", "Copyright (c) 2026 zcli");
        if let Err(err) = res.compile() {
            // 资源嵌入失败不应阻断构建（rc.exe 缺失等环境问题）
            println!("cargo:warning=Windows 资源嵌入失败: {err}");
        }
    }
}
