# pdfxml - PDF XML 注释导出工具

[![Rust](https://img.shields.io/badge/Rust-1.75+-orange?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

将 XFDF/XML 格式的 PDF 注释导出为 PDF 文件的 Rust 工具。

## ✨ 功能特性

- 📝 **完整支持 XFDF 格式**：解析 Adobe 标准的 XML Forms Data Format
- 🎨 **多种注释类型**：
  - 文本注释（Text/便签）
  - 高亮（Highlight）、下划线（Underline）、删除线（StrikeOut）
  - 自由文本（FreeText）
  - 方形（Square）、圆形（Circle）
  - 线条（Line）、多边形（Polygon）
  - 墨迹/手绘（Ink）
  - 图章（Stamp）
  - 弹出窗口（Popup）
  
- 🔧 **两种导出模式**：
  1. 创建新 PDF：仅包含注释信息
  2. 合并到现有 PDF：将注释添加到已有文档

- ⚡ **高性能**：纯 Rust 实现，内存安全，速度快

## 📖 XFDF 格式简介

XFDF (XML Forms Data Format) 是 Adobe 定义的一种 XML 格式，用于独立存储 PDF 文档中的表单数据和注释。它是 FDF 的 XML 版本。

### 基本结构示例

```xml
<?xml version="1.0" encoding="UTF-8" ?>
<xfdf xmlns="http://ns.adobe.com/xfdf/" xml:space="preserve">
  <annots>
    <text subject="注释标题"
          page="0"
          rect="100,700,300,750"
          title="作者名称"
          date="D:20240101120000+08'00'"
          color="#FFFF00">
      注释内容...
    </text>
  </annots>
</xfdf>
```

### 支持的注释类型

| XML 元素 | PDF 子类型 | 说明 |
|---------|-----------|------|
| `<text>` | Text | 文本便签 |
| `<highlight>` | Highlight | 高亮文本 |
| `<underline>` | Underline | 下划线 |
| `<strikeout>` | StrikeOut | 删除线 |
| `<freetext>` | FreeText | 自由文本框 |
| `<square>` | Square | 矩形标记 |
| `<circle>` | Circle | 圆形/椭圆标记 |
| `<line>` | Line | 箭头/线条 |
| `<polygon>` | Polygon | 多边形区域 |
| `<ink>` | Ink | 手绘墨迹 |
| `<stamp>` | Stamp | 预设图章 |

## 🚀 快速开始

### 安装

```bash
# 克隆项目
git clone https://github.com/your-repo/pdfxml.git
cd pdfxml

# 编译
cargo build --release
```

### 使用方法

```bash
# 基本用法：创建新 PDF
pdfxml -i input.xfdf -o output.pdf

# 指定目标 PDF（合并注释到现有文档）
pdfxml -i annotations.xfdf --target-pdf original.pdf -o annotated.pdf

# 详细输出模式
pdfxml -i input.xfdf -o output.pdf -v
```

### 命令行参数

| 参数 | 缩写 | 必填 | 说明 |
|-----|------|------|------|
| `--input` | `-i` | ✓ | 输入的 XFDF/XML 文件路径 |
| `--output` | `-o` | | 输出的 PDF 文件路径（默认与输入同名） |
| `--target-pdf` | `-t` | | 目标 PDF 文件路径（可选） |
| `--verbose` | `-v` | | 详细输出模式 |

## 📁 项目结构

```
pdfxml/
├── Cargo.toml              # 项目配置和依赖
├── README.md               # 项目说明
├── src/
│   ├── main.rs            # 主程序入口和 CLI
│   ├── xfdf.rs             # XFDF/XML 解析模块
│   ├── pdf.rs              # PDF 生成模块
│   ├── annotation.rs       # 注释数据结构定义
│   └── error.rs            # 错误类型定义
├── examples/
│   ├── sample.xfdf         # 完整示例文件
│   └── minimal.xfdf        # 最小示例文件
└── tests/
    └── integration_test.rs # 集成测试
```

## 💡 使用示例

### 示例 1：从 XFDF 创建新 PDF

```bash
pdfxml -i examples/sample.xfdf -o output/sample.pdf
```

### 示例 2：合并注释到现有 PDF

假设你有一个 PDF 和一个包含注释的 XFDF 文件：

```bash
pdfxml -i comments.xfdf --target-pdf document.pdf -o annotated_document.pdf
```

### 示例 3：在代码中使用库

```rust
use pdfxml::{XfdfDocument, PdfAnnotationExporter};
use std::fs;

fn main() -> anyhow::Result<()> {
    // 读取并解析 XFDF 文件
    let xml_content = fs::read_to_string("annotations.xfdf")?;
    let xfdf_doc = XfdfDocument::parse(&xml_content)?;
    
    println!("找到 {} 条注释", xfdf_doc.annotations.len());
    
    // 导出到新 PDF
    let exporter = PdfAnnotationExporter::new();
    exporter.export_to_new_pdf(&xfdf_doc, "output.pdf")?;
    
    Ok(())
}
```

## 🧪 运行测试

```bash
# 运行所有测试
cargo test

# 显示详细输出
cargo test -- --nocapture

# 仅运行单元测试
cargo test lib::

# 运行集成测试
cargo test test::
```

## 🛠️ 技术实现

### 核心依赖库

| 库 | 用途 | 版本 |
|---|------|------|
| [quick-xml](https://github.com/tafia/quick-xml) | 高性能 XML 解析 | ^0.36 |
| [lopdf](https://github.com/J-F-Liu/lopdf) | PDF 生成和操作 | ^0.35 |
| [clap](https://github.com/clap-rs/clap) | 命令行参数解析 | ^4.5 |
| [chrono](https://docs.rs/chrono) | 日期时间处理 | ^0.4 |

### 架构设计

```
XFDF/XML 文件
     │
     ▼
┌─────────────┐    ┌─────────────────┐
│  xfdf.rs     │───▶│ annotation.rs   │
│  XML 解析器  │    │  数据结构定义    │
└─────────────┘    └─────────────────┘
                          │
                          ▼
                   ┌─────────────┐
                   │  pdf.rs      │
                   │  PDF 生成器   │
                   └─────────────┘
                          │
                          ▼
                    PDF 文件输出
```

## 📋 待办功能

- [ ] 支持 FDF（非 XML）格式
- [ ] 支持富文本内容（HTML/XHTML）
- [ ] 支持附件注释
- [ ] 支持声音注释
- [ ] 支持链接注释
- [ ] 支持表单字段填充
- [ ] GUI 界面
- [ ] WebAssembly 编译支持

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！

1. Fork 本仓库
2. 创建特性分支 (`git checkout -b feature/AmazingFeature`)
3. 提交更改 (`git commit -m 'Add some AmazingFeature'`)
4. 推送到分支 (`git push origin feature/AmazingFeature`)
5. 开启 Pull Request

## 📄 许可证

本项目采用 MIT 许可证 - 查看 [LICENSE](LICENSE) 文件了解详情。

## 🙏 致谢

- Adobe XFDF 规范
- lopdf 库的维护者
- Rust 社区

---

**Made with ❤️ using Rust**
