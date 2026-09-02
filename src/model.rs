//! 数据模型与纯函数工具：大纲提取、目录扫描。

use gpui_component::tree::TreeItem;
use std::path::Path;

/// 大纲条目：从 Markdown 文本中提取的一个标题。
#[derive(Clone, Debug)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    /// 0 基行号，用于编辑器跳转。
    pub line: u32,
}

/// 提取 Markdown 标题列表（跳过 fenced code block 内出现的伪标题）。
/// 支持 ATX（`# 标题`）与 Setext（下划线 `===` / `---`）两种形式。
pub fn extract_headings(src: &str) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut in_fence: Option<char> = None;
    let lines: Vec<&str> = src.lines().collect();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // fenced code block 开关：``` 或 ~~~（允许最多 3 个前导空格）
        if let Some(fence) = fence_open(line) {
            in_fence = match in_fence {
                Some(f) if f == fence => None, // 同字符围栏 → 关闭
                Some(_) => in_fence,           // 不同围栏字符，忽略该行
                None => Some(fence),           // 未在围栏内 → 打开
            };
            i += 1;
            continue;
        }

        if in_fence.is_none() {
            if let Some((level, text)) = atx_heading(line) {
                headings.push(Heading {
                    level,
                    text,
                    line: i as u32,
                });
            } else if !line.trim().is_empty() && i + 1 < lines.len() {
                // Setext 标题：当前行是文本，下一行是 === 或 ---
                if let Some(level) = setext_level(lines[i + 1]) {
                    headings.push(Heading {
                        level,
                        text: line.trim().to_string(),
                        line: i as u32,
                    });
                    i += 1; // 跳过下划线行
                }
            }
        }
        i += 1;
    }
    headings
}

/// 识别围栏行（``` 或 ~~~），返回围栏字符。
fn fence_open(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    if trimmed.len() < 3 {
        return None;
    }
    if trimmed.starts_with("```") {
        Some('`')
    } else if trimmed.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

/// 识别 ATX 标题行，返回 (级别, 标题文本)。
fn atx_heading(line: &str) -> Option<(u8, String)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    // '#' 后必须是空格或行尾
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some((hashes as u8, text))
}

/// 识别 Setext 下划线行，返回标题级别。
fn setext_level(line: &str) -> Option<u8> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|c| c == '=') {
        Some(1)
    } else if trimmed.chars().all(|c| c == '-') {
        Some(2)
    } else {
        None
    }
}

/// 目录树的跨线程中间表示：全部字段 `Send`，可在后台线程扫描后传回主线程。
#[derive(Clone)]
pub struct DirNode {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub disabled: bool,
    pub children: Vec<DirNode>,
}

/// 在后台线程扫描目录（返回 Send 的中间表示，避免 TreeItem 中的 Rc 跨线程）。
pub fn scan_node(root: &Path) -> Option<DirNode> {
    if !root.is_dir() {
        return None;
    }
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let mut children = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        // 目录在前，按名称排序
        entries.sort_by(|a, b| {
            let a_dir = a.path().is_dir();
            let b_dir = b.path().is_dir();
            b_dir
                .cmp(&a_dir)
                .then_with(|| a.file_name().cmp(&b.file_name()))
        });
        for entry in entries {
            let child_path = entry.path();
            let child_name = entry.file_name().to_string_lossy().into_owned();
            // 跳过隐藏项与常见的无关目录
            if child_name.starts_with('.')
                || matches!(child_name.as_str(), "target" | "node_modules")
            {
                continue;
            }
            if child_path.is_dir() {
                if let Some(node) = scan_node(&child_path) {
                    children.push(node);
                }
            } else {
                children.push(DirNode {
                    path: child_path.to_string_lossy().into_owned(),
                    name: child_name,
                    is_dir: false,
                    disabled: child_path.extension().and_then(|e| e.to_str()) != Some("md"),
                    children: Vec::new(),
                });
            }
        }
    }
    Some(DirNode {
        path: root.to_string_lossy().into_owned(),
        name,
        is_dir: true,
        disabled: false,
        children,
    })
}

/// 把扫描结果（中间表示）转换为 TreeItem 列表（主线程构建）。
/// 所有目录默认折叠（点击展开）；非 .md 文件置为禁用。
pub fn nodes_to_items(nodes: Vec<DirNode>) -> Vec<TreeItem> {
    nodes.into_iter().filter_map(node_to_item).collect()
}

fn node_to_item(node: DirNode) -> Option<TreeItem> {
    if node.is_dir {
        let mut item = TreeItem::new(node.path, node.name);
        for child in node.children {
            if let Some(child_item) = node_to_item(child) {
                item = item.child(child_item);
            }
        }
        Some(item)
    } else {
        let mut item = TreeItem::new(node.path, node.name);
        if node.disabled {
            item = item.disabled(true);
        }
        Some(item)
    }
}

/// 在嵌套的树节点中按 id（完整路径）递归查找节点。
pub fn find_item<'a>(items: &'a [TreeItem], id: &str) -> Option<&'a TreeItem> {
    for item in items {
        if item.id == id {
            return Some(item);
        }
        if let Some(found) = find_item(&item.children, id) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_headings_mixed() {
        let src = "\
# 一级标题
正文

## 二级标题
### 三级标题

```rust
# 代码块里的伪标题（不应被提取）
```
# 围栏后的标题

Setext 标题
===

- 普通列表
";
        let headings = extract_headings(src);
        let lines: Vec<String> = headings
            .iter()
            .map(|h| format!("#{} {}", h.level, h.text))
            .collect();
        assert_eq!(
            lines,
            vec![
                "#1 一级标题",
                "#2 二级标题",
                "#3 三级标题",
                "#1 围栏后的标题",
                "#1 Setext 标题"
            ]
        );
        // 行号验证：Setext 标题行是 "Setext 标题"（0 基第 11 行）
        assert_eq!(headings.last().unwrap().line, 11);
    }

    #[test]
    fn extract_headings_empty_and_short() {
        assert!(extract_headings("").is_empty());
        assert!(extract_headings("普通文本\n没有标题").is_empty());
        // 只有 '#' 无内容不算标题
        assert!(extract_headings("#\n## 有内容").len() == 1);
        // 7 个 # 不是标题
        assert!(extract_headings("####### 过多").is_empty());
    }
}
