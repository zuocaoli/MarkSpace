# Markdown Workspace

[![CI](https://github.com/zuocaoli/MarkSpace/actions/workflows/ci.yml/badge.svg)](https://github.com/zuocaoli/MarkSpace/actions/workflows/ci.yml)
[![Release](https://github.com/zuocaoli/MarkSpace/actions/workflows/release.yml/badge.svg)](https://github.com/zuocaoli/MarkSpace/actions/workflows/release.yml)

基于 [GPUI](https://gpui.rs) 的原生 Markdown 桌面工作台：目录管理、Markdown 编辑、实时预览与文档导航 —— 接近 VS Code 的 Markdown Mode 体验，但**轻量、快速、无 Electron**。

## 功能特性

- **目录管理**：左侧目录树，后台线程扫描、只读浏览（展开/折叠、点击打开、当前文件选中高亮、手动刷新、右键重命名文件/目录并自动同步打开的文档）
- **多文档编辑**：中心编辑器面板内置 Tab 栏，同时打开多个文档；纯文本编辑（v1 不含语法高亮），带行号、文本搜索、脏状态标记（`●`）
- **实时预览**：编辑后 300ms 防抖渲染 Markdown → GPUI 原生元素（标题/列表/引用/代码块/表格/行内样式/链接/分割线），滚动位置在连续编辑间保持
- **大纲导航**：从标题自动提取文档大纲（ATX 与 Setext 形式，自动跳过代码围栏内的伪标题），点击跳转到编辑器对应位置
- **保存**：`Ctrl+S` 保存，关闭未保存文档时有确认对话框；状态栏显示当前文档路径、修改状态与字符数
- **中文界面**：全部 UI 文案为中文

## 构建与运行

`gpui`/`gpui_platform` 为 `zed` 仓库的 git 依赖（由 `Cargo.lock` 锁定 commit），首次构建会克隆并编译这些依赖，耗时较长；`gpui-component` 使用本地 vendored 副本（`vendor/`，内含 MarkSpace 对 dock 省略号菜单的移除补丁）。

```bash
cargo check          # 快速类型检查
cargo run            # 打开当前目录
cargo run -- <目录>  # 打开指定工作目录
cargo test           # 运行单元测试（大纲提取等纯函数）
```

### 系统要求

- Linux / macOS / Windows（GPUI 跨平台）
- 中文渲染依赖系统 CJK 字体。若界面出现占位方块（豆腐块），请安装中文字体，例如：

  ```bash
  # Fedora / RHEL
  sudo dnf install google-noto-sans-cjk-fonts
  # Debian / Ubuntu
  sudo apt install fonts-noto-cjk
  ```

## CI/CD

GitHub Actions 自动完成质量门禁与发布：

- **CI**（`.github/workflows/ci.yml`）：main 推送、所有 PR 及 `v*` tag 触发；Linux / macOS / Windows 三平台执行 `cargo check` + `cargo test`，Linux 额外执行 `rustfmt` 与 `clippy -D warnings`（格式/裁剪结果与平台无关，只跑一遍避免依赖树编译三遍）
- **Release**（`.github/workflows/release.yml`）：推送 `v*` tag 时三平台 `--release` 构建并归档（Linux 为 tar.gz，macOS / Windows 为 zip），汇总发布到 [Releases](https://github.com/zuocaoli/MarkSpace/releases/latest) 页面（附自动生成的更新说明）

## 安装

无需本地编译，直接下载对应平台的预编译二进制（文件名中的版本号随版本变化，以 [Releases](https://github.com/zuocaoli/MarkSpace/releases/latest) 页面实际为准）：

| 平台 | 文件 | 说明 |
| --- | --- | --- |
| Linux x86_64 | `MarkSpace-0.1.0-x86_64-unknown-linux-gnu.tar.gz` | `tar xzf` 解压后运行 `./MarkSpace` |
| macOS（Apple Silicon） | `MarkSpace-0.1.0-aarch64-apple-darwin.zip` | 解压后运行 `MarkSpace`；未签名，首次运行请右键 → 打开，或 `xattr -d com.apple.quarantine MarkSpace` |
| Windows x86_64 | `MarkSpace-0.1.0-x86_64-pc-windows-msvc.zip` | 解压后双击 `MarkSpace.exe`；未签名，SmartScreen 拦截时点「更多信息 → 仍要运行」 |

> 也可通过源码构建：`cargo install --git <仓库地址>` 或按上文「构建与运行」章节本地编译。

## 快捷键

| 按键 | 功能 |
| --- | --- |
| `Ctrl+S` | 保存当前文档 |
| `Ctrl+Shift+V` | 切换右侧「预览 / 大纲」面板显示 |
| 状态栏左侧图标按钮 | 切换左/右侧面板显示 |

## 项目结构

```
src/
├── main.rs          # 入口：CLI 参数、全局按键绑定、开窗（Root 包裹）
├── workspace.rs     # Workspace 根视图：Dock 三栏布局、全局动作、状态栏、通知层
├── model.rs         # 数据模型与纯函数：大纲提取、目录扫描
├── tree_panel.rs    # 目录树面板（浏览 + 右键重命名）
├── editor_panel.rs  # 编辑器面板（多文档 Tab、脏跟踪、编辑/预览切换、防抖预览、保存）
├── outline_panel.rs # 大纲面板（标题提取与跳转）
├── settings.rs      # 设置框架：配置模型 + 持久化 + 设置对话框（字体/字号/缩放）
└── paper_theme.rs   # 纸质感主题：暖米白「书籍纸」色板，覆盖默认浅色主题
```

## 技术要点

- **零新增依赖**：Markdown 解析（markdown-rs）、文本编辑器、虚拟化目录树、Dock 布局全部复用 `gpui-component` 既有能力，未引入任何新的第三方 crate。
- **面板间通信**：各面板持有目标实体的弱引用（`WeakEntity`），通过直接调用与 `notify` 协作，无自建消息总线。
- **实时预览**：每个文档持有一个 `Entity<TextViewState>`（预览实体），编辑器内容变化时经 300ms 防抖调用 `set_text` 更新——同一实体使滚动位置得以保留。
- **目录扫描**：`TreeItem` 内含 `Rc` 无法跨线程，因此先在后台线程扫描为 `Send` 的中间表示（`DirNode`），回到主线程再转换为树节点，避免阻塞 UI。
- **编辑器状态持久化**：每个文档的编辑器/预览都是独立 `Entity`，切换 Tab 不丢光标、滚动与撤销栈。多文档用面板内部 TabBar 管理，而非 Dock 动态面板（Dock 无公开的「激活已有面板」API）。

## 已知取舍与路线图

**v1 已取舍**：编辑器为纯文本（无语法高亮）；目录树无新建/删除；无编辑器↔预览滚动同步；无外部文件变更监听；窗口使用默认标题栏。

**规划中的后续迭代**：

- Markdown 语法高亮（`gpui-component` 内置 tree-sitter + 高亮器，一行开关即可接入）
- 目录树内新建 / 删除文件，并自动刷新
- `Ctrl+P` 快速打开（`gpui-component::command` 命令面板）
- 编辑器 ↔ 预览滚动同步与标题锚点跳转
- 外部文件变更监听（依赖树已含有 `notify`）
- 自定义 `TitleBar`

## License

[MIT](LICENSE)